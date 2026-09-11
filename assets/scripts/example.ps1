# @lumen.title Copy IP Address
# @lumen.subtitle Example script command
# @lumen.mode copy
# @lumen.keywords network, ip, address
#
# ---------------------------------------------------------------------------
# This is a Lumen script command (tier T1). Any executable that Lumen can run
# is a command: the headers above are read when the folder is indexed, and the
# file itself is only ever run when you pick its row in the launcher.
#
#   @lumen.title      Required. What appears in the list.
#   @lumen.subtitle   Optional. The second line.
#   @lumen.keywords   Optional. Comma separated; matched but never shown.
#   @lumen.mode       silent (default) -- run it, ignore the output.
#                     copy             -- run it, put the output on the clipboard.
#
# The comment character is whatever your language uses. Lumen looks for the
# @lumen. marker and does not care what precedes it, so .ps1, .cmd, .sh and .js
# all work the same way.
#
# Delete this file once you have your own.
# ---------------------------------------------------------------------------

(Get-NetIPAddress -AddressFamily IPv4 |
    Where-Object { $_.InterfaceAlias -notmatch 'Loopback' } |
    Select-Object -First 1 -ExpandProperty IPAddress)
