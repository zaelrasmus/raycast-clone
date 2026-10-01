# Rebuild the bundled catalogue of places from GeoNames (F13).
#
# The catalogue is committed, not downloaded at build time: a build that
# depends on a third party's server being up is a build that fails for
# reasons nobody can fix. This script is how the committed files are made,
# so they can be reproduced rather than taken on trust, and refreshed when
# GeoNames is.
#
#   powershell -File scripts/make-places.ps1
#
# GeoNames is CC BY 4.0: the attribution is in the README and in the view.
#
# Writes two files to assets/places/:
#
#   countries.tsv   code, name, continent, population -- one line per
#                   country, small enough to read in a diff.
#   cities.tsv.gz   one line per city over 15,000 people, largest first:
#                   name, ASCII name (empty when the same), country code,
#                   region, region in ASCII (empty when the same), IANA zone,
#                   latitude, longitude, population.

param(
    [string] $Out = (Join-Path $PSScriptRoot '..\assets\places')
)

$ErrorActionPreference = 'Stop'
$base = 'https://download.geonames.org/export/dump'
$work = Join-Path ([IO.Path]::GetTempPath()) 'lumen-places'
New-Item -ItemType Directory -Force -Path $work, $Out | Out-Null

foreach ($file in 'cities15000.zip', 'countryInfo.txt', 'admin1CodesASCII.txt') {
    Invoke-WebRequest -Uri "$base/$file" -OutFile (Join-Path $work $file) -UseBasicParsing
}
Expand-Archive -Force -Path (Join-Path $work 'cities15000.zip') -DestinationPath $work

$utf8 = [Text.UTF8Encoding]::new($false)
$invariant = [Globalization.CultureInfo]::InvariantCulture

# Countries: code, name, continent, population. Comment lines start with
# '#'. The population ranks `korea`: South before North.
$countries = foreach ($line in [IO.File]::ReadAllLines((Join-Path $work 'countryInfo.txt'), $utf8)) {
    if ($line.StartsWith('#') -or $line.Trim() -eq '') { continue }
    $f = $line.Split("`t")
    "$($f[0])`t$($f[4])`t$($f[8])`t$($f[7])"
}
[IO.File]::WriteAllText((Join-Path $Out 'countries.tsv'), (($countries | Sort-Object) -join "`n") + "`n", $utf8)

# Regions, by `CC.code`: the name and its ASCII spelling.
$regions = @{}
foreach ($line in [IO.File]::ReadAllLines((Join-Path $work 'admin1CodesASCII.txt'), $utf8)) {
    $f = $line.Split("`t")
    $regions[$f[0]] = @($f[1], $f[2])
}

$cities = foreach ($line in [IO.File]::ReadAllLines((Join-Path $work 'cities15000.txt'), $utf8)) {
    $f = $line.Split("`t")
    $name = $f[1]
    $ascii = if ($f[2] -ceq $name) { '' } else { $f[2] }
    $country = $f[8]
    $region = $regions["$country.$($f[10])"]
    $regionName = if ($region) { $region[0] } else { '' }
    $regionAscii = if ($region -and $region[1] -cne $region[0]) { $region[1] } else { '' }
    $population = [long]('0' + $f[14])
    [pscustomobject]@{
        Population = $population
        Line = @(
            $name, $ascii, $country, $regionName, $regionAscii, $f[17],
            ([double]$f[4]).ToString('0.###', $invariant),
            ([double]$f[5]).ToString('0.###', $invariant),
            $population
        ) -join "`t"
    }
}

# Largest first, so the loader's order is already the ranking's tiebreak; by
# the line after that, so the file is the same every time it is made.
$sorted = $cities | Sort-Object -Property @{ Expression = 'Population'; Descending = $true }, Line
$text = $utf8.GetBytes((($sorted | ForEach-Object Line) -join "`n") + "`n")

$target = Join-Path $Out 'cities.tsv.gz'
$stream = [IO.File]::Create($target)
try {
    $gzip = [IO.Compression.GZipStream]::new($stream, [IO.Compression.CompressionLevel]::Optimal)
    $gzip.Write($text, 0, $text.Length)
    $gzip.Dispose()
} finally {
    $stream.Dispose()
}

"{0} countries, {1} cities, {2:N0} bytes of text, {3:N0} compressed" -f `
    $countries.Count, $sorted.Count, $text.Length, (Get-Item $target).Length
