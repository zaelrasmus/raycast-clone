<#
.SYNOPSIS
    Draw Lumen's application icon.

.DESCRIPTION
    Writes assets/icon/lumen.ico, which is embedded in the executable and is
    what Windows shows in Task Manager, the Start Menu and the notification
    area.

    Drawn in code rather than committed as a binary somebody has to open a
    design tool to change: the mark is a rounded square in the accent colour
    with a four-pointed spark on it, and every number that decides how it looks
    is below, next to the reason for it.

    The palette is the theme's, not a new one. `accent` is Nocturne's
    #7C6BD6 -- the same colour the launcher highlights a match in -- so the icon
    in the tray and the interface it opens are recognisably the same thing.
#>

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent $PSScriptRoot
$outDir = Join-Path $root 'assets\icon'
$outFile = Join-Path $outDir 'lumen.ico'
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

# The theme's accent, and a slightly lighter top for a gradient that reads as
# depth at 256 px and as nothing at all at 16 px -- which is the right way
# round.
$accentTop = [System.Drawing.Color]::FromArgb(255, 0x94, 0x85, 0xE4)
$accentBottom = [System.Drawing.Color]::FromArgb(255, 0x6B, 0x59, 0xC9)
$spark = [System.Drawing.Color]::FromArgb(255, 0xFF, 0xFF, 0xFF)

# Windows 11 rounds application icons at about 22% of their width. Matching it
# means the icon sits correctly beside the system's own rather than looking
# like a sticker.
$cornerFraction = 0.22

# How far the spark reaches from the centre, as a fraction of the icon. Short
# of the edge on purpose: an icon drawn to its own edges reads as larger than
# its neighbours, which is the same optical rule as the glyph set.
$sparkFraction = 0.30

# How much the arms pinch. 0 is a diamond, 1 is a cross of hairlines. 0.16 is a
# spark that still has body at 16 px.
$pinch = 0.16

function New-Frame([int]$size) {
    $bitmap = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bitmap)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality

    # -- The rounded square --------------------------------------------------
    $inset = [math]::Max(1.0, $size * 0.03)
    $box = New-Object System.Drawing.RectangleF($inset, $inset, ($size - 2 * $inset), ($size - 2 * $inset))
    $radius = $box.Width * $cornerFraction

    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $radius * 2
    $path.AddArc($box.X, $box.Y, $d, $d, 180, 90)
    $path.AddArc(($box.Right - $d), $box.Y, $d, $d, 270, 90)
    $path.AddArc(($box.Right - $d), ($box.Bottom - $d), $d, $d, 0, 90)
    $path.AddArc($box.X, ($box.Bottom - $d), $d, $d, 90, 90)
    $path.CloseFigure()

    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
        (New-Object System.Drawing.PointF($box.X, $box.Y)),
        (New-Object System.Drawing.PointF($box.X, $box.Bottom)),
        $accentTop, $accentBottom)
    $g.FillPath($brush, $path)
    $brush.Dispose()
    $path.Dispose()

    # -- The spark -----------------------------------------------------------
    #
    # Four arms, each a Bezier from one tip to the next with both control
    # points pulled toward the centre. Pulling them in is what makes the arms
    # concave; how far decides whether it reads as a star or as a diamond.
    $cx = $size / 2.0
    $cy = $size / 2.0
    $r = $size * $sparkFraction
    $c = $r * $pinch

    $tips = @(
        @( 0, -1), @( 1, 0), @( 0, 1), @(-1, 0)
    )

    $sparkPath = New-Object System.Drawing.Drawing2D.GraphicsPath
    for ($i = 0; $i -lt 4; $i++) {
        $from = $tips[$i]
        $to = $tips[($i + 1) % 4]

        $p0 = New-Object System.Drawing.PointF(($cx + $from[0] * $r), ($cy + $from[1] * $r))
        $p3 = New-Object System.Drawing.PointF(($cx + $to[0] * $r), ($cy + $to[1] * $r))
        $c1 = New-Object System.Drawing.PointF(($cx + $from[0] * $c), ($cy + $from[1] * $c))
        $c2 = New-Object System.Drawing.PointF(($cx + $to[0] * $c), ($cy + $to[1] * $c))

        $sparkPath.AddBezier($p0, $c1, $c2, $p3)
    }
    $sparkPath.CloseFigure()

    $sparkBrush = New-Object System.Drawing.SolidBrush($spark)
    $g.FillPath($sparkBrush, $sparkPath)
    $sparkBrush.Dispose()
    $sparkPath.Dispose()

    $g.Dispose()
    return $bitmap
}

# -- Assemble the .ico ------------------------------------------------------
#
# Small sizes as device-independent bitmaps, large ones as PNG, and the split
# is not arbitrary. Windows has accepted PNG-compressed icon entries since
# Vista, but plenty of code that reads icons has not caught up -- .NET's own
# `System.Drawing.Icon.ToBitmap` throws on them, which is how this was found.
# Anything that old code is likely to ask for is written the old way.

function ConvertTo-Dib([System.Drawing.Bitmap]$bitmap) {
    $size = $bitmap.Width
    $stream = New-Object System.IO.MemoryStream
    $writer = New-Object System.IO.BinaryWriter($stream)

    # BITMAPINFOHEADER. The doubled height is the format's oldest quirk: an
    # icon bitmap holds the colour image and a 1-bit mask stacked, and the
    # header describes both.
    $writer.Write([uint32]40)
    $writer.Write([int32]$size)
    $writer.Write([int32]($size * 2))
    $writer.Write([uint16]1)
    $writer.Write([uint16]32)
    $writer.Write([uint32]0)          # BI_RGB
    $writer.Write([uint32]0)          # size, may be zero for BI_RGB
    $writer.Write([int32]0); $writer.Write([int32]0)
    $writer.Write([uint32]0); $writer.Write([uint32]0)

    # The colour image, bottom-up, as BGRA.
    $rect = New-Object System.Drawing.Rectangle(0, 0, $size, $size)
    $locked = $bitmap.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly,
        [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $stride = [int]$locked.Stride
    # `[byte[]]::new` rather than `New-Object byte[] (...)`: the latter reads a
    # computed size as a constructor argument list and produces something that
    # is not the array you asked for, which shows up much later as an icon
    # file Windows will not open.
    $pixels = [byte[]]::new($stride * $size)
    [System.Runtime.InteropServices.Marshal]::Copy($locked.Scan0, $pixels, 0, $pixels.Length)
    $bitmap.UnlockBits($locked)

    for ($y = $size - 1; $y -ge 0; $y--) {
        $writer.Write($pixels, ($y * $stride), ($size * 4))
    }

    # The AND mask. Left entirely zero: with a 32-bit image Windows composites
    # from the alpha channel, and a mask that disagreed with it would show as a
    # hard edge around everything.
    $maskStride = [int][math]::Ceiling($size / 32.0) * 4
    $writer.Write([byte[]]::new($maskStride * $size))

    $writer.Flush()
    $bytes = $stream.ToArray()
    $writer.Dispose()
    $stream.Dispose()

    # The unary comma is load-bearing. `return $bytes` hands the array to the
    # pipeline, which unrolls it into 1128 separate bytes and reassembles them
    # as an Object[]; the caller then writes something that is not the array it
    # asked for, and the icon file is the right length in its directory and the
    # wrong length on disk.
    return , $bytes
}

$sizes = @(16, 24, 32, 48, 64, 128, 256)
$frames = @()
foreach ($size in $sizes) {
    $bitmap = New-Frame $size

    if ($size -le 48) {
        $bytes = ConvertTo-Dib $bitmap
    } else {
        $stream = New-Object System.IO.MemoryStream
        $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
        $bytes = $stream.ToArray()
        $stream.Dispose()
    }

    $frames += , @{ Size = $size; Bytes = $bytes }
    $bitmap.Dispose()
}

$out = New-Object System.IO.MemoryStream
$writer = New-Object System.IO.BinaryWriter($out)

# ICONDIR: reserved, type 1 (icon), count.
$writer.Write([uint16]0)
$writer.Write([uint16]1)
$writer.Write([uint16]$frames.Count)

# ICONDIRENTRY is 16 bytes each, so the first image starts after all of them.
$offset = 6 + 16 * $frames.Count
foreach ($frame in $frames) {
    # 256 is written as 0: the field is one byte and 256 does not fit.
    $dimension = if ($frame.Size -ge 256) { 0 } else { $frame.Size }
    $writer.Write([byte]$dimension)   # width
    $writer.Write([byte]$dimension)   # height
    $writer.Write([byte]0)            # palette entries, 0 for truecolour
    $writer.Write([byte]0)            # reserved
    $writer.Write([uint16]1)          # colour planes
    $writer.Write([uint16]32)         # bits per pixel
    $writer.Write([uint32]$frame.Bytes.Length)
    $writer.Write([uint32]$offset)
    $offset += $frame.Bytes.Length
}

# Cast at the call site too, so the overload chosen is Write(byte[]) and not
# whatever a loosely typed array would bind to.
foreach ($frame in $frames) { $writer.Write([byte[]]$frame.Bytes) }

$writer.Flush()
[System.IO.File]::WriteAllBytes($outFile, $out.ToArray())
$writer.Dispose()
$out.Dispose()

# -- Check it ---------------------------------------------------------------
#
# Including the lengths, because the failure above was a directory that
# described a file larger than the one it was in.
#
# An icon file that is subtly wrong does not fail: it silently falls back to a
# generic document glyph somewhere in Windows, weeks later. Reading every
# bitmap entry back through the same loader that threw on PNG entries is the
# cheapest proof that this one is well formed.

$declared = 6 + 16 * $frames.Count + ($frames | ForEach-Object { $_.Bytes.Length } | Measure-Object -Sum).Sum
$actual = (Get-Item $outFile).Length
if ($declared -ne $actual) {
    throw "the directory describes $declared bytes and the file is $actual"
}

foreach ($frame in $frames) {
    if ($frame.Size -gt 48) { continue }
    try {
        $icon = New-Object System.Drawing.Icon($outFile, $frame.Size, $frame.Size)
        $check = $icon.ToBitmap()
        if ($check.Width -ne $frame.Size) {
            throw "asked for $($frame.Size) and got $($check.Width)"
        }
        $check.Dispose()
        $icon.Dispose()
    } catch {
        throw "the $($frame.Size) px entry is malformed: $_"
    }
}

$kb = [math]::Round((Get-Item $outFile).Length / 1KB, 1)
Write-Host "Wrote $outFile ($kb KB, $($frames.Count) sizes, all readable)" -ForegroundColor Green
