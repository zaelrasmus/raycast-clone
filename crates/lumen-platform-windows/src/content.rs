//! Content search through Windows Search (F16, slice 6): the files whose
//! contents match, asked of the system's own index over its OLE DB provider,
//! whether that index covers a folder, and its settings.

use std::ffi::{OsString, c_void};
use std::os::windows::ffi::OsStringExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, CoCreateInstance};
use windows::Win32::System::Search::{
    CSearchManager, DBACCESSOR_ROWDATA, DBBINDING, DBMEMOWNER_CLIENTOWNED, DBPARAMIO_NOTPARAM,
    DBPART_LENGTH, DBPART_STATUS, DBPART_VALUE, DBSTATUS_S_OK, DBTYPE_WSTR, HACCESSOR, IAccessor,
    ICommandText, IDBCreateCommand, IDBCreateSession, IDBInitialize, IDataInitialize, IRowset,
    ISearchManager, MSDAINITIALIZE,
};
use windows::core::{GUID, HSTRING, IUnknown, Interface as _, w};

use crate::com;

/// The default command dialect, `DBGUID_DEFAULT` in oledb.h. The `windows`
/// crate does not carry it.
const DBGUID_DEFAULT: GUID = GUID::from_u128(0xc8b521fb_5cf3_11ce_ade5_00aa0044773d);

/// The longest path read back, in UTF-16 units. A longer one comes back
/// truncated and is left out rather than reported wrong.
const PATH_UNITS: usize = 4096;

/// The SQL that asks the index for `limit` files under `scopes` whose
/// contents match every word of `words` as a prefix, or `None` when there is
/// nothing to ask: no words, no scopes or no room.
///
/// Words are split on whitespace. A `"` would end the word's phrase early, so
/// it is dropped; a `'` would end the string, so it is doubled, in words and
/// scopes alike. Folders are left out: a folder's name is not its contents.
fn query(words: &str, scopes: &[PathBuf], limit: usize) -> Option<String> {
    let words: Vec<String> = words
        .split_whitespace()
        .map(|word| word.replace('"', "").replace('\'', "''"))
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}*\""))
        .collect();
    if words.is_empty() || scopes.is_empty() || limit == 0 {
        return None;
    }
    let scopes: Vec<String> = scopes
        .iter()
        .map(|scope| {
            let path = scope.to_string_lossy().replace('\\', "/");
            format!("SCOPE='file:{}'", path.replace('\'', "''"))
        })
        .collect();
    Some(format!(
        "SELECT TOP {limit} System.ItemPathDisplay FROM SystemIndex \
         WHERE ({}) AND System.ItemType <> 'Directory' AND CONTAINS(*,'{}')",
        scopes.join(" OR "),
        words.join(" AND "),
    ))
}

/// The files under `scopes` whose contents match `words`, at most `limit`.
pub(crate) fn search(words: &str, scopes: &[PathBuf], limit: usize) -> Result<Vec<PathBuf>> {
    let Some(sql) = query(words, scopes, limit) else {
        return Ok(Vec::new());
    };
    let _apartment = com::Apartment::enter()?;
    let rowset = execute(&sql).context("asking Windows Search; is its service running?")?;
    read_paths(&rowset, limit)
}

/// Run `sql` against the index and return its rows.
fn execute(sql: &str) -> Result<IRowset> {
    // SAFETY: plain COM calls on interfaces this function holds; every out
    // parameter is a local that outlives the call.
    unsafe {
        let init: IDataInitialize = CoCreateInstance(&MSDAINITIALIZE, None, CLSCTX_INPROC_SERVER)
            .context("creating the OLE DB service")?;
        let mut source: Option<IUnknown> = None;
        init.GetDataSource(
            None,
            CLSCTX_INPROC_SERVER.0,
            w!("Provider=Search.CollatorDSO.1;Extended Properties='Application=Windows'"),
            &IDBInitialize::IID,
            &mut source,
        )
        .context("opening the Windows Search provider")?;
        let source: IDBInitialize = source.context("no Windows Search provider")?.cast()?;
        source
            .Initialize()
            .context("connecting to Windows Search")?;
        let session: IDBCreateCommand = source
            .cast::<IDBCreateSession>()?
            .CreateSession(None, &IDBCreateCommand::IID)?
            .cast()?;
        let command: ICommandText = session.CreateCommand(None, &ICommandText::IID)?.cast()?;
        command.SetCommandText(&DBGUID_DEFAULT, &HSTRING::from(sql))?;
        let mut rowset: Option<IUnknown> = None;
        command
            .Execute(None, &IRowset::IID, None, None, Some(&mut rowset))
            .context("running the content query")?;
        Ok(rowset
            .context("the content query returned no rows")?
            .cast()?)
    }
}

/// One row as the accessor writes it: the path column's status, its length
/// in bytes, and its characters, terminated.
#[repr(C)]
struct Row {
    status: u32,
    length: usize,
    value: [u16; PATH_UNITS],
}

/// The paths in the first column of `rowset`, at most `limit`.
fn read_paths(rowset: &IRowset, limit: usize) -> Result<Vec<PathBuf>> {
    let accessor: IAccessor = rowset.cast()?;
    let binding = DBBINDING {
        iOrdinal: 1,
        obValue: std::mem::offset_of!(Row, value),
        obLength: std::mem::offset_of!(Row, length),
        obStatus: std::mem::offset_of!(Row, status),
        dwPart: (DBPART_VALUE.0 | DBPART_LENGTH.0 | DBPART_STATUS.0) as u32,
        dwMemOwner: DBMEMOWNER_CLIENTOWNED.0 as u32,
        eParamIO: DBPARAMIO_NOTPARAM.0 as u32,
        cbMaxLen: PATH_UNITS * size_of::<u16>(),
        wType: DBTYPE_WSTR.0 as u16,
        ..Default::default()
    };
    let mut handle = HACCESSOR::default();
    // SAFETY: one binding, read from a local for the length of the call.
    unsafe {
        accessor.CreateAccessor(
            DBACCESSOR_ROWDATA.0 as u32,
            1,
            &binding,
            size_of::<Row>(),
            &mut handle,
            None,
        )?
    };
    let paths = read_rows(rowset, handle, limit);
    // SAFETY: the handle was created above and is released once.
    unsafe { accessor.ReleaseAccessor(handle, None)? };
    paths
}

fn read_rows(rowset: &IRowset, accessor: HACCESSOR, limit: usize) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut row = Box::new(Row {
        status: 0,
        length: 0,
        value: [0; PATH_UNITS],
    });
    while paths.len() < limit {
        // One row a call: the provider writes its handle into `hrow`.
        let mut hrow = 0usize;
        let mut hrows: *mut usize = &mut hrow;
        let mut obtained = 0usize;
        // SAFETY: `hrows` points at room for the one row asked for.
        unsafe { rowset.GetNextRows(0, 0, &mut obtained, std::slice::from_mut(&mut hrows))? };
        if obtained == 0 {
            break;
        }
        // SAFETY: `row` is laid out as the accessor binds it, and the row
        // handle is released once whether or not its data was read.
        let read =
            unsafe { rowset.GetData(hrow, accessor, (&mut *row as *mut Row).cast::<c_void>()) };
        unsafe {
            rowset.ReleaseRows(
                1,
                &hrow,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )?
        };
        read?;
        if row.status == DBSTATUS_S_OK.0 as u32 {
            let units = (row.length / size_of::<u16>()).min(PATH_UNITS);
            paths.push(PathBuf::from(OsString::from_wide(&row.value[..units])));
        }
    }
    Ok(paths)
}

/// The URL the crawl scope manager knows a folder by: `file:///` and the
/// path with its own separators, ending in one, as in Microsoft's
/// "Managing Scope Rules" (`file:///C:\ProjectFiles\`).
fn scope_url(scope: &Path) -> String {
    let path = scope.to_string_lossy();
    format!("file:///{}\\", path.trim_end_matches(['\\', '/']))
}

/// Whether Windows Search's own catalog covers `scope`.
pub(crate) fn covered(scope: &Path) -> Result<bool> {
    let _apartment = com::Apartment::enter()?;
    // SAFETY: plain COM calls on interfaces this function holds. The manager
    // lives in the indexer's own process, so it is a local server, not an
    // in-process one.
    unsafe {
        let manager: ISearchManager = CoCreateInstance(&CSearchManager, None, CLSCTX_LOCAL_SERVER)
            .context("asking Windows Search; is its service running?")?;
        let included = manager
            .GetCatalog(w!("SystemIndex"))?
            .GetCrawlScopeManager()?
            .IncludedInCrawlScope(&HSTRING::from(scope_url(scope)))
            .with_context(|| format!("asking whether {} is indexed", scope.display()))?;
        Ok(included.as_bool())
    }
}

/// Open the Control Panel's Indexing Options.
pub(crate) fn open_indexing_options() -> Result<()> {
    Command::new("control.exe")
        .arg("srchadmin.dll")
        .spawn()
        .context("opening Indexing Options")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn asks_for_every_word_as_a_prefix_under_every_scope() {
        assert_eq!(
            query("budget report", &scopes(&[r"C:\Users\x", r"D:\Dev"]), 20).as_deref(),
            Some(
                "SELECT TOP 20 System.ItemPathDisplay FROM SystemIndex \
                 WHERE (SCOPE='file:C:/Users/x' OR SCOPE='file:D:/Dev') \
                 AND System.ItemType <> 'Directory' \
                 AND CONTAINS(*,'\"budget*\" AND \"report*\"')"
            ),
        );
    }

    #[test]
    fn one_scope_and_one_word() {
        assert_eq!(
            query("  invoice\t", &scopes(&[r"C:\Users\x"]), 5).as_deref(),
            Some(
                "SELECT TOP 5 System.ItemPathDisplay FROM SystemIndex \
                 WHERE (SCOPE='file:C:/Users/x') \
                 AND System.ItemType <> 'Directory' \
                 AND CONTAINS(*,'\"invoice*\"')"
            ),
        );
    }

    #[test]
    fn doubles_single_quotes_in_words_and_scopes() {
        let sql = query("o'neil", &scopes(&[r"C:\Users\o'brien"]), 5).unwrap();
        assert!(sql.contains("SCOPE='file:C:/Users/o''brien'"), "{sql}");
        assert!(sql.contains("CONTAINS(*,'\"o''neil*\"')"), "{sql}");
    }

    #[test]
    fn drops_double_quotes_from_words() {
        let sql = query("\"say\" hel\"lo \"", &scopes(&[r"C:\x"]), 5).unwrap();
        assert!(
            sql.ends_with("CONTAINS(*,'\"say*\" AND \"hello*\"')"),
            "{sql}"
        );
    }

    #[test]
    fn nothing_to_ask_is_no_query() {
        let home = scopes(&[r"C:\Users\x"]);
        assert_eq!(query("", &home, 5), None);
        assert_eq!(query(" \t ", &home, 5), None);
        assert_eq!(query("\" \"\"", &home, 5), None);
        assert_eq!(query("word", &[], 5), None);
        assert_eq!(query("word", &home, 0), None);
    }

    #[test]
    fn the_crawl_scope_url_ends_in_a_separator() {
        assert_eq!(scope_url(Path::new(r"C:\Users\x")), r"file:///C:\Users\x\");
        assert_eq!(scope_url(Path::new(r"C:\Users\x\")), r"file:///C:\Users\x\");
        assert_eq!(scope_url(Path::new(r"D:\")), r"file:///D:\");
    }

    /// Asks the real index; run by hand with `--ignored`.
    #[test]
    #[ignore]
    fn asks_the_live_index() {
        let home = PathBuf::from(std::env::var_os("USERPROFILE").expect("USERPROFILE"));
        let covered = covered(&home).expect("coverage");
        let found = search("the", std::slice::from_ref(&home), 5).expect("search");
        println!("home covered: {covered}; matches: {}", found.len());
    }
}
