# World Clock — the first extension written against Lumen's protocol.
#
# PowerShell on purpose. An extension is whatever the author already has
# installed, and the protocol is a line of JSON in and a line of JSON out
# precisely so that "whatever they already have" is enough. There is no SDK to
# install and nothing to compile: this file is the whole extension.
#
# The conversation:
#
#   host -> {"type":"run","command":"times"}
#   here -> {"type":"render","view":{...}}
#   host -> {"type":"event","event":{"type":"action-invoked",...}}
#   here -> {"type":"request","id":1,"capability":"copy-to-clipboard","text":"..."}
#   host -> {"type":"response","id":1,"result":{"status":"ok"}}
#
# Everything this wants done goes back through the host as a capability request,
# even though a script could obviously set the clipboard itself. That is AD-09:
# the boundary is drawn now so that a sandbox later is a change of runtime
# rather than a redesign.

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::InputEncoding  = [System.Text.UTF8Encoding]::new($false)

$zones = @(
    @{ Id = 'utc';       Label = 'UTC';       Zone = 'UTC' }
    @{ Id = 'london';    Label = 'London';    Zone = 'GMT Standard Time' }
    @{ Id = 'madrid';    Label = 'Madrid';    Zone = 'Romance Standard Time' }
    @{ Id = 'new-york';  Label = 'New York';  Zone = 'Eastern Standard Time' }
    @{ Id = 'sao-paulo'; Label = 'Sao Paulo'; Zone = 'E. South America Standard Time' }
    @{ Id = 'bengaluru'; Label = 'Bengaluru'; Zone = 'India Standard Time' }
    @{ Id = 'tokyo';     Label = 'Tokyo';     Zone = 'Tokyo Standard Time' }
    @{ Id = 'sydney';    Label = 'Sydney';    Zone = 'AUS Eastern Standard Time' }
)

function Send-Line($object) {
    # Depth matters: a view is nested five levels deep and PowerShell truncates
    # at two by default, which produces a message the host parses happily and
    # draws as nothing at all.
    $json = $object | ConvertTo-Json -Depth 12 -Compress
    [Console]::Out.WriteLine($json)
    [Console]::Out.Flush()
}

function Get-Rows {
    $now = [DateTime]::UtcNow
    $here = [TimeZoneInfo]::Local

    $items = foreach ($zone in $zones) {
        try {
            $info = [TimeZoneInfo]::FindSystemTimeZoneById($zone.Zone)
        } catch {
            continue
        }

        $local = [TimeZoneInfo]::ConvertTimeFromUtc($now, $info)
        $offset = $info.GetUtcOffset($now) - $here.GetUtcOffset($now)

        $relative = if ($offset.TotalMinutes -eq 0) { 'same as here' }
                    elseif ($offset.TotalMinutes -gt 0) { '{0:+0.#} h' -f $offset.TotalHours }
                    else { '{0:0.#} h' -f $offset.TotalHours }

        # Working hours, roughly. A tone is a meaning and the theme decides what
        # it looks like — an extension cannot name a colour, by design.
        $tone = if ($local.Hour -ge 9 -and $local.Hour -lt 18) { 'positive' }
                elseif ($local.Hour -ge 7 -and $local.Hour -lt 22) { 'neutral' }
                else { 'caution' }

        @{
            id          = $zone.Id
            title       = $local.ToString('HH:mm')
            subtitle    = $zone.Label
            icon        = @{ type = 'text'; text = '🕑' }
            keywords    = @($zone.Label, $zone.Zone)
            accessories = @(
                @{ type = 'text'; text = $local.ToString('ddd d MMM') }
                @{ type = 'tag'; text = $relative; tone = $tone }
            )
            actions     = @(
                @{ id = 'copy';     title = 'Copy the Time'; shortcut = 'Ctrl+C' }
                @{ id = 'copy-iso'; title = 'Copy as ISO 8601' }
            )
        }
    }

    return @($items)
}

function Send-View {
    Send-Line @{
        type = 'render'
        view = @{
            type        = 'list'
            placeholder = 'Search cities...'
            filtering   = 'host'
            sections    = @(@{ title = 'Now'; items = Get-Rows })
        }
    }
}

function Get-TimeFor($id, $iso) {
    $zone = $zones | Where-Object { $_.Id -eq $id } | Select-Object -First 1
    if (-not $zone) { return $null }

    $info = [TimeZoneInfo]::FindSystemTimeZoneById($zone.Zone)
    $local = [TimeZoneInfo]::ConvertTimeFromUtc([DateTime]::UtcNow, $info)

    if ($iso) { return $local.ToString('yyyy-MM-ddTHH:mm:ss') }
    return "$($local.ToString('HH:mm')) in $($zone.Label)"
}

$requestId = 0

# Blocking reads on stdin. The host keeps the pipe open for as long as the view
# is up and closes it when the user leaves, which is how this process learns to
# stop without anything having to kill it.
while ($true) {
    $line = [Console]::In.ReadLine()
    if ($null -eq $line) { break }
    if ([string]::IsNullOrWhiteSpace($line)) { continue }

    try { $message = $line | ConvertFrom-Json } catch { continue }

    switch ($message.type) {
        'run' { Send-View }

        'event' {
            $event = $message.event
            switch ($event.type) {
                'action-invoked' {
                    $text = Get-TimeFor $event.item ($event.action -eq 'copy-iso')
                    if ($text) {
                        $requestId++
                        Send-Line @{
                            type       = 'request'
                            id         = $requestId
                            capability = 'copy-to-clipboard'
                            text       = $text
                        }
                        Send-Line @{ type = 'toast'; message = "Copied $text"; tone = 'positive' }
                    }
                    Send-Line @{ type = 'close' }
                }
                'dismissed' { return }
                default { }
            }
        }

        'response' {
            # Either it worked or the host already logged why it did not.
        }

        default { }
    }
}
