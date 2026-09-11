# Characters — the second extension.
#
# The plan is specific about why this exists: "Write the second extension before
# stabilising anything in the protocol. The first extension lies: it fits
# perfectly because the protocol was designed while looking at it."
#
# So this one is deliberately a different shape from World Clock. It does its
# own filtering rather than letting the host do it, it re-renders on every
# keystroke instead of rendering once, it has an empty state, and it stays open
# after an action instead of closing. Between them the two cover every message
# in the protocol except `detail`.
#
# The table is written as codepoints rather than as literal characters, and that
# is not tidiness: Windows PowerShell's tokenizer accepts the Unicode quotation
# marks U+2018, U+2019, U+201C and U+201D as *string delimiters*, so a literal
# right single quote inside single quotes ends the string early and the file
# stops parsing several lines later. Codepoints also make the U+XXXX accessory
# true by construction rather than by arithmetic on whatever is in the string.

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::InputEncoding  = [System.Text.UTF8Encoding]::new($false)

$characters = @(
    @{ Code = 0x2014; Name = 'Em dash';                Tags = 'dash punctuation long' }
    @{ Code = 0x2013; Name = 'En dash';                Tags = 'dash punctuation range' }
    @{ Code = 0x2026; Name = 'Horizontal ellipsis';    Tags = 'dots punctuation' }
    @{ Code = 0x201C; Name = 'Left double quote';      Tags = 'quote curly smart' }
    @{ Code = 0x201D; Name = 'Right double quote';     Tags = 'quote curly smart' }
    @{ Code = 0x2018; Name = 'Left single quote';      Tags = 'quote curly smart' }
    @{ Code = 0x2019; Name = 'Right single quote';     Tags = 'quote curly apostrophe' }
    @{ Code = 0x2192; Name = 'Rightwards arrow';       Tags = 'arrow right' }
    @{ Code = 0x2190; Name = 'Leftwards arrow';        Tags = 'arrow left' }
    @{ Code = 0x2191; Name = 'Upwards arrow';          Tags = 'arrow up' }
    @{ Code = 0x2193; Name = 'Downwards arrow';        Tags = 'arrow down' }
    @{ Code = 0x23CE; Name = 'Return symbol';          Tags = 'enter key return' }
    @{ Code = 0x2318; Name = 'Place of interest sign'; Tags = 'command key mac' }
    @{ Code = 0x2325; Name = 'Option key';             Tags = 'alt option key mac' }
    @{ Code = 0x21E7; Name = 'Upwards white arrow';    Tags = 'shift key' }
    @{ Code = 0x00B7; Name = 'Middle dot';             Tags = 'separator bullet punctuation' }
    @{ Code = 0x2022; Name = 'Bullet';                 Tags = 'list bullet point' }
    @{ Code = 0x00D7; Name = 'Multiplication sign';    Tags = 'times multiply maths' }
    @{ Code = 0x00F7; Name = 'Division sign';          Tags = 'divide maths' }
    @{ Code = 0x2248; Name = 'Almost equal to';        Tags = 'approximately maths' }
    @{ Code = 0x2260; Name = 'Not equal to';           Tags = 'inequality maths' }
    @{ Code = 0x00B0; Name = 'Degree sign';            Tags = 'temperature angle' }
    @{ Code = 0x00B1; Name = 'Plus-minus sign';        Tags = 'tolerance maths' }
    @{ Code = 0x00A9; Name = 'Copyright sign';         Tags = 'legal mark' }
    @{ Code = 0x00AE; Name = 'Registered sign';        Tags = 'legal trademark' }
    @{ Code = 0x2122; Name = 'Trade mark sign';        Tags = 'legal trademark' }
    @{ Code = 0x00A7; Name = 'Section sign';           Tags = 'legal reference' }
    @{ Code = 0x00B6; Name = 'Pilcrow sign';           Tags = 'paragraph typography' }
    @{ Code = 0x2020; Name = 'Dagger';                 Tags = 'footnote typography' }
    # Not a character, so it carries its text instead of a codepoint. The
    # accessory says so rather than inventing a U+ for it.
    @{ Code = 0; Text = ([char]0x00AF + '\_(' + [char]0x30C4 + ')_/' + [char]0x00AF)
       Name = 'Shrug'; Tags = 'kaomoji emoticon shrug' }
)

foreach ($character in $characters) {
    if (-not $character.Text) {
        $character.Text = [char]::ConvertFromUtf32($character.Code)
    }
    $character.Label = if ($character.Code -gt 0) { 'U+{0:X4}' -f $character.Code } else { 'kaomoji' }
}

function Send-Line($object) {
    $json = $object | ConvertTo-Json -Depth 12 -Compress
    [Console]::Out.WriteLine($json)
    [Console]::Out.Flush()
}

function Get-Matches($query) {
    if ([string]::IsNullOrWhiteSpace($query)) { return @($characters) }

    $needle = $query.Trim().ToLowerInvariant()
    return @($characters | Where-Object {
        $_.Name.ToLowerInvariant().Contains($needle) -or
        $_.Tags.Contains($needle) -or
        $_.Label.ToLowerInvariant().Contains($needle) -or
        $_.Text -eq $query
    })
}

function Send-View($query, $copied) {
    $items = foreach ($character in (Get-Matches $query)) {
        $accessories = @(@{ type = 'text'; text = $character.Label })
        if ($copied -and $copied -eq $character.Name) {
            $accessories += @{ type = 'tag'; text = 'copied'; tone = 'positive' }
        }

        @{
            id          = $character.Name
            title       = $character.Name
            subtitle    = $character.Tags
            icon        = @{ type = 'text'; text = $character.Text }
            accessories = $accessories
            actions     = @(
                @{ id = 'copy';      title = 'Copy the Character'; shortcut = 'Ctrl+C' }
                @{ id = 'copy-name'; title = 'Copy the Name' }
            )
        }
    }

    Send-Line @{
        type = 'render'
        view = @{
            type        = 'list'
            placeholder = 'Search characters...'
            # The other half of the pair: this extension filters, so the host
            # sends it every keystroke and shows the list exactly as it arrives.
            filtering   = 'extension'
            empty       = @{
                title       = 'No character by that name'
                description = 'Try arrow, quote, dash or maths.'
                icon        = @{ type = 'glyph'; name = 'snippet' }
            }
            sections    = @(@{ items = @($items) })
        }
    }
}

$requestId = 0
$query = ''

while ($true) {
    $line = [Console]::In.ReadLine()
    if ($null -eq $line) { break }
    if ([string]::IsNullOrWhiteSpace($line)) { continue }

    try { $message = $line | ConvertFrom-Json } catch { continue }

    switch ($message.type) {
        'run' { Send-View $query $null }

        'event' {
            $event = $message.event
            switch ($event.type) {
                'query-changed' {
                    $query = $event.query
                    Send-View $query $null
                }
                'action-invoked' {
                    $character = $characters | Where-Object { $_.Name -eq $event.item } | Select-Object -First 1
                    if ($character) {
                        $text = if ($event.action -eq 'copy-name') { $character.Name } else { $character.Text }
                        $requestId++
                        Send-Line @{
                            type       = 'request'
                            id         = $requestId
                            capability = 'copy-to-clipboard'
                            text       = $text
                        }
                        # Deliberately does *not* close. Copying several
                        # characters in a row is the whole use, and an extension
                        # that decides for itself when the view goes away is an
                        # extension the user ends up arguing with.
                        Send-View $query $character.Name
                    }
                }
                'dismissed' { return }
                default { }
            }
        }

        'response' { }
        default { }
    }
}
