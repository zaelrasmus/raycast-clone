# Rebuild the bundled catalogue of emoji and symbols (F18).
#
# Committed, not downloaded at build time, for the reason make-places.ps1
# gives: a build must not depend on a third party's server being up. This
# script is how the committed file is made, so it can be reproduced rather
# than taken on trust, and refreshed when Unicode is.
#
#   powershell -File scripts/make-emoji.ps1
#
# Every source is pinned to a release, so the same script makes the same
# bytes: Emoji 15.1, CLDR 45 (the release that names Emoji 15.1) and the
# Unicode Character Database 15.1.
#
#   emoji-test.txt    which sequences are emoji, their group, subgroup and
#                     order. Fully-qualified only. Skin-tone variants are
#                     left out (F18 ships none) but remembered on their base,
#                     for the deferred decision. Country and subdivision
#                     flags are left out too: Windows draws neither, and the
#                     Fluent images have none.
#   CLDR annotations  the English name (`type="tts"`) and keywords of each
#                     emoji, and keywords for many symbols.
#   UnicodeData.txt   the names of the symbols, which are not emoji: arrows,
#                     maths, currency, letterlike, geometric shapes,
#                     punctuation, technical and box drawing.
#
# Writes assets/emoji/emoji.tsv.gz, one entry per line, in catalogue order --
# the emoji in Unicode's order, then the symbols by category and code point:
#
#   chars, category, subgroup, name, keywords joined by '|', skin tones (1/0)
#
# This file is ASCII on purpose: Windows PowerShell 5.1 reads a script with
# no byte-order mark in the ANSI codepage. Characters are built from code
# points instead.

param(
    [string] $Out = (Join-Path $PSScriptRoot '..\assets\emoji')
)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$sources = [ordered]@{
    'emoji-test.txt'   = 'https://unicode.org/Public/emoji/15.1/emoji-test.txt'
    'UnicodeData.txt'  = 'https://unicode.org/Public/15.1.0/ucd/UnicodeData.txt'
    'annotations.xml'  = 'https://raw.githubusercontent.com/unicode-org/cldr/release-45/common/annotations/en.xml'
    'derived.xml'      = 'https://raw.githubusercontent.com/unicode-org/cldr/release-45/common/annotationsDerived/en.xml'
}
$work = Join-Path ([IO.Path]::GetTempPath()) 'lumen-emoji'
New-Item -ItemType Directory -Force -Path $work, $Out | Out-Null
foreach ($file in $sources.Keys) {
    Invoke-WebRequest -Uri $sources[$file] -OutFile (Join-Path $work $file) -UseBasicParsing
}

$utf8 = [Text.UTF8Encoding]::new($false)
$fe0f = [string][char]0xFE0F

function Chars([int[]] $codepoints) {
    -join ($codepoints | ForEach-Object { [char]::ConvertFromUtf32($_) })
}

# A field must not break the line it is on.
function Clean([string] $text) {
    $text -replace '[\t\r\n|]', ' '
}

# CLDR, by the emoji with U+FE0F stripped: the spoken name, and the rest.
# Ordinal: PowerShell's own hashtable compares keys by culture, under which
# many symbols are equal -- it named the heavy plus sign *Georgian Lari*.
function Ordinal { ,[Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal) }
function Lookup($table, [string] $key) {
    $value = $null
    if ($table.TryGetValue($key, [ref] $value)) { $value } else { $null }
}
$names = Ordinal
$keywords = Ordinal
foreach ($file in 'annotations.xml', 'derived.xml') {
    [xml] $xml = [IO.File]::ReadAllText((Join-Path $work $file), $utf8)
    foreach ($node in $xml.ldml.annotations.annotation) {
        $key = $node.cp.Replace($fe0f, '')
        if ($node.type -eq 'tts') {
            $names[$key] = $node.InnerText.Trim()
        } else {
            $keywords[$key] = @($node.InnerText.Split('|') | ForEach-Object { $_.Trim() } | Where-Object { $_ })
        }
    }
}

# The keywords of an entry: CLDR's, then `extra`, without its name and
# without repeats, in the order they came.
function Keywords([string] $key, [string] $name, [string[]] $extra) {
    $seen = [Collections.Generic.HashSet[string]]::new()
    [void] $seen.Add($name)
    $list = foreach ($word in @(Lookup $keywords $key) + $extra) {
        if ($word -and $seen.Add($word)) { Clean $word }
    }
    @($list) -join '|'
}

$groups = @{
    'Smileys & Emotion' = 'smileys-people'
    'People & Body'     = 'smileys-people'
    'Animals & Nature'  = 'animals-nature'
    'Food & Drink'      = 'food-drink'
    'Travel & Places'   = 'travel-places'
    'Activities'        = 'activities'
    'Objects'           = 'objects'
    'Symbols'           = 'symbols'
    'Flags'             = 'flags'
}
$skipSubgroups = @('country-flag', 'subdivision-flag')

# First pass: every sequence emoji-test.txt lists, whatever its status, so
# a symbol that is also an emoji is not listed twice; and the bases that
# have skin-tone variants.
$emoji = [Collections.Generic.List[object]]::new()
$isEmoji = [Collections.Generic.HashSet[int]]::new()
$toned = [Collections.Generic.HashSet[string]]::new()
$group = ''
$subgroup = ''
$pattern = '^([0-9A-F ]+?)\s*;\s*([a-z-]+)\s*#\s*\S+\s+E\d+\.\d+\s+(.+)$'
foreach ($line in [IO.File]::ReadAllLines((Join-Path $work 'emoji-test.txt'), $utf8)) {
    if ($line -match '^# group: (.+)$') { $group = $Matches[1].Trim(); continue }
    if ($line -match '^# subgroup: (.+)$') { $subgroup = $Matches[1].Trim(); continue }
    if ($line -notmatch $pattern) { continue }
    $codepoints = @($Matches[1].Split(' ', [StringSplitOptions]::RemoveEmptyEntries) | ForEach-Object { [Convert]::ToInt32($_, 16) })
    $status = $Matches[2]
    $testName = $Matches[3].Trim()

    $plain = @($codepoints | Where-Object { $_ -ne 0xFE0F })
    if ($plain.Count -eq 1) { [void] $isEmoji.Add($plain[0]) }

    $modifiers = @($codepoints | Where-Object { $_ -ge 0x1F3FB -and $_ -le 0x1F3FF })
    if ($modifiers.Count -gt 0) {
        if ($codepoints.Count -gt $modifiers.Count) {
            [void] $toned.Add((Chars @($plain | Where-Object { $_ -lt 0x1F3FB -or $_ -gt 0x1F3FF })))
        }
        continue
    }
    if ($status -ne 'fully-qualified') { continue }
    if (-not $groups.ContainsKey($group)) { continue }
    if ($skipSubgroups -contains $subgroup) { continue }
    $emoji.Add([pscustomobject]@{
        Chars = Chars $codepoints
        Key = Chars $plain
        Category = $groups[$group]
        Subgroup = $subgroup
        Fallback = $testName
    })
}

$lines = [Collections.Generic.List[string]]::new()
foreach ($entry in $emoji) {
    $name = Lookup $names $entry.Key
    if (-not $name) { $name = $entry.Fallback }
    $name = Clean $name
    $skin = if ($toned.Contains($entry.Key)) { '1' } else { '0' }
    $lines.Add((@($entry.Chars, $entry.Category, $entry.Subgroup, $name, (Keywords $entry.Key $name @()), $skin) -join "`t"))
}
$emojiCount = $lines.Count

# The symbols, by category in the order the catalogue lists them. Each range
# is a block, or the part of one worth listing; its slug is the subgroup.
# A range with a filter keeps only the code points whose name matches it.
$symbols = @(
    @('arrows', 'arrows', 0x2190, 0x21FF, $null),
    @('arrows', 'supplemental-arrows-a', 0x27F0, 0x27FF, $null),
    @('arrows', 'supplemental-arrows-b', 0x2900, 0x297F, $null),
    @('arrows', 'miscellaneous-symbols-and-arrows', 0x2B00, 0x2B2F, 'ARROW'),
    @('math', 'latin-1-supplement', @(0x00AC, 0x00B1, 0x00D7, 0x00F7), $null, $null),
    @('math', 'mathematical-operators', 0x2200, 0x22FF, $null),
    @('currency', 'latin-1-supplement', @(0x0024, 0x00A2, 0x00A3, 0x00A4, 0x00A5), $null, $null),
    @('currency', 'currency-symbols', 0x20A0, 0x20C0, $null),
    @('letterlike', 'letterlike-symbols', 0x2100, 0x214F, $null),
    @('geometric-shapes', 'geometric-shapes', 0x25A0, 0x25FF, $null),
    @('punctuation', 'latin-1-supplement', @(0x00A1, 0x00A7, 0x00A9, 0x00AB, 0x00AE, 0x00B0, 0x00B6, 0x00B7, 0x00BB, 0x00BF), $null, $null),
    @('punctuation', 'general-punctuation', 0x2010, 0x2027, $null),
    @('punctuation', 'general-punctuation', 0x2030, 0x205E, $null),
    @('technical', 'miscellaneous-technical', 0x2300, 0x23FF, $null),
    @('box-drawing', 'box-drawing', 0x2500, 0x257F, $null)
)

# UnicodeData: name and general category by code point. Every code point in
# the ranges above is listed singly, so ranges (`<CJK Ideograph, First>`)
# need no handling.
$unicode = @{}
foreach ($line in [IO.File]::ReadAllLines((Join-Path $work 'UnicodeData.txt'), $utf8)) {
    $f = $line.Split(';')
    $unicode[[Convert]::ToInt32($f[0], 16)] = @($f[1], $f[2])
}

foreach ($range in $symbols) {
    $category, $block, $first, $last, $filter = $range
    $codepoints = if ($first -is [array]) { $first } else { $first..$last }
    foreach ($codepoint in $codepoints) {
        $data = $unicode[$codepoint]
        # Unassigned, a control, a format character or a space: nothing to
        # see in a grid.
        if (-not $data -or $data[1] -match '^[CZ]' -or $data[0].StartsWith('<')) { continue }
        if ($isEmoji.Contains($codepoint)) { continue }
        if ($filter -and $data[0] -notmatch $filter) { continue }
        $chars = Chars @($codepoint)
        $name = Clean $data[0].ToLowerInvariant()
        $spoken = @(Lookup $names $chars) | Where-Object { $_ }
        $lines.Add((@($chars, $category, $block, $name, (Keywords $chars $name $spoken), '0') -join "`t"))
    }
}

$text = $utf8.GetBytes(($lines -join "`n") + "`n")

# .NET's gzip header carries no file name and a zero timestamp, so the same
# text compresses to the same bytes.
$target = Join-Path $Out 'emoji.tsv.gz'
$stream = [IO.File]::Create($target)
try {
    $gzip = [IO.Compression.GZipStream]::new($stream, [IO.Compression.CompressionLevel]::Optimal)
    $gzip.Write($text, 0, $text.Length)
    $gzip.Dispose()
} finally {
    $stream.Dispose()
}

"{0} emoji, {1} symbols, {2:N0} bytes of text, {3:N0} compressed" -f `
    $emojiCount, ($lines.Count - $emojiCount), $text.Length, (Get-Item $target).Length
