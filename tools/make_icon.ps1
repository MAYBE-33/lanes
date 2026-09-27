# Draw the application mark and write the icon files.
#
# Run from the repository root:
#     powershell -ExecutionPolicy Bypass -File tools\make_icon.ps1
#
# Outputs, all committed so no build step depends on this script running:
#   assets/icon.ico    the Windows icon, 16 through 256
#   assets/icon.png    256x256, for documentation and the README
#   assets/tray.png    16x16, the source of the tray icon's pixels
#
# THE MARK
#
# Three fader bars of different heights, in three of the channel accents, on a
# dark rounded tile. It is the product drawn literally, which is the right
# choice for a tray icon: it has to be identifiable at sixteen pixels in a row
# of thirty other things, and an abstract mark loses that fight every time.
#
# The tile matters as much as the bars. Without a background the bars are three
# disconnected slivers that disappear against a light taskbar; the tile gives
# the mark an edge and a constant contrast whatever is behind it.

param([string]$OutDir = "assets")

Add-Type -AssemblyName System.Drawing

$ground = [System.Drawing.Color]::FromArgb(255, 0x17, 0x17, 0x1B)
$edge   = [System.Drawing.Color]::FromArgb(255, 0x2E, 0x2E, 0x36)
$bars = @(
  @{ Color = [System.Drawing.Color]::FromArgb(255, 0xD9, 0x77, 0x57); Height = 0.52 }  # Game
  @{ Color = [System.Drawing.Color]::FromArgb(255, 0x6B, 0xA8, 0xC9); Height = 0.80 }  # Chat
  @{ Color = [System.Drawing.Color]::FromArgb(255, 0x7F, 0xB0, 0x69); Height = 0.36 }  # Media
)

function New-Mark([int]$size) {
  $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = 'AntiAlias'
  $g.Clear([System.Drawing.Color]::Transparent)

  # The tile. At 16px a rounded rect degenerates into mush, so below 24 it is
  # a plain square with the corner pixels left off - which reads as rounded at
  # that size and stays crisp.
  $r = [Math]::Max(2, [Math]::Round($size * 0.22))
  $tile = New-Object System.Drawing.Drawing2D.GraphicsPath
  if ($size -ge 24) {
    $d = $r * 2
    $tile.AddArc(0, 0, $d, $d, 180, 90)
    $tile.AddArc($size - $d - 1, 0, $d, $d, 270, 90)
    $tile.AddArc($size - $d - 1, $size - $d - 1, $d, $d, 0, 90)
    $tile.AddArc(0, $size - $d - 1, $d, $d, 90, 90)
    $tile.CloseFigure()
  } else {
    $tile.AddRectangle((New-Object System.Drawing.Rectangle 0, 0, ($size - 1), ($size - 1)))
  }

  $g.FillPath((New-Object System.Drawing.SolidBrush $ground), $tile)
  if ($size -ge 32) {
    $g.DrawPath((New-Object System.Drawing.Pen $edge, ([float]([Math]::Max(1, $size / 64)))), $tile)
  }

  # Three bars, evenly spaced, rising from a common baseline.
  $pad      = $size * 0.20
  $inner    = $size - 2 * $pad
  $barW     = $inner / 5.0          # three bars and two gaps of the same width
  $baseline = $size - $pad
  $radius   = [Math]::Max(1, $barW / 2)

  for ($i = 0; $i -lt 3; $i++) {
    $x = $pad + $i * 2 * $barW
    $h = $inner * $bars[$i].Height
    $y = $baseline - $h
    $brush = New-Object System.Drawing.SolidBrush $bars[$i].Color

    if ($size -ge 32) {
      $path = New-Object System.Drawing.Drawing2D.GraphicsPath
      $d = $radius * 2
      $path.AddArc($x, $y, $d, $d, 180, 90)
      $path.AddArc($x + $barW - $d, $y, $d, $d, 270, 90)
      $path.AddArc($x + $barW - $d, $y + $h - $d, $d, $d, 0, 90)
      $path.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
      $path.CloseFigure()
      $g.FillPath($brush, $path)
    } else {
      # Whole pixels at small sizes. A half-pixel bar is a grey smear.
      $g.FillRectangle($brush,
        [int][Math]::Round($x), [int][Math]::Round($y),
        [int][Math]::Max(2, [Math]::Round($barW)), [int][Math]::Round($h))
    }
  }

  $g.Dispose()
  return $bmp
}

$dir = Join-Path (Get-Location) $OutDir
New-Item -ItemType Directory -Force -Path $dir | Out-Null

# --- Raster the mark at every size ------------------------------------------
$sizes = 16, 24, 32, 48, 64, 128, 256
$marks = @{}
foreach ($s in $sizes) { $marks[$s] = New-Mark $s }

# --- PNGs for documentation and for the tray --------------------------------
function Save-Png($bmp, $path) {
  $ms = New-Object System.IO.MemoryStream
  $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
  [System.IO.File]::WriteAllBytes($path, $ms.ToArray())
  $ms.Dispose()
}
Save-Png $marks[256] (Join-Path $dir 'icon.png')
Save-Png $marks[16]  (Join-Path $dir 'tray.png')

# --- Raw pixels for the tray ------------------------------------------------
#
# `tray-icon` wants an RGBA buffer, and the core has no image decoder - nor any
# reason to grow one for a single 32x32 picture. Writing the pixels out flat
# means the core can `include_bytes!` them and hand them straight over: no
# decoding, no crate, and nothing to go missing at run time, which is what the
# specification's "one executable, nothing to install" asks for.
$tray = $marks[32]
$rect = New-Object System.Drawing.Rectangle 0, 0, 32, 32
$data = $tray.LockBits($rect, 'ReadOnly', ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb))
$rgba = New-Object byte[] (32 * 32 * 4)
try {
  $row = New-Object byte[] (32 * 4)
  for ($y = 0; $y -lt 32; $y++) {
    [System.Runtime.InteropServices.Marshal]::Copy(
      [IntPtr]::Add($data.Scan0, $y * $data.Stride), $row, 0, $row.Length)
    # GDI+ gives BGRA; the tray wants RGBA.
    for ($x = 0; $x -lt 32; $x++) {
      $i = $x * 4
      $o = ($y * 32 + $x) * 4
      $rgba[$o + 0] = $row[$i + 2]
      $rgba[$o + 1] = $row[$i + 1]
      $rgba[$o + 2] = $row[$i + 0]
      $rgba[$o + 3] = $row[$i + 3]
    }
  }
} finally { $tray.UnlockBits($data) }
[System.IO.File]::WriteAllBytes((Join-Path $dir 'tray-32.rgba'), $rgba)

# --- ICO --------------------------------------------------------------------
#
# Written by hand, because .NET has no multi-size ICO writer.
#
# The encoding per size is not a free choice. An ICO entry may hold either a
# PNG or a bottom-up DIB, and while Windows itself has accepted PNG entries
# since Vista, `System.Drawing.Icon.ToBitmap` still cannot decode one - it
# throws "Requested range extends past the end of the array", which is a long
# way from "this frame is a PNG". Anything that reads the icon through
# System.Drawing therefore sees a broken file.
#
# So the small sizes are DIBs, which everything can read, and only 256 is a
# PNG, where the size saving is worth having and where no small-icon API looks.
function Get-Dib($bmp) {
  $w = $bmp.Width; $h = $bmp.Height
  $ms = New-Object System.IO.MemoryStream
  $bw = New-Object System.IO.BinaryWriter $ms

  # BITMAPINFOHEADER. The height is doubled because the entry carries the
  # colour bitmap and an AND mask stacked together, even at 32bpp where the
  # mask is redundant and must still be present.
  $bw.Write([UInt32]40)
  $bw.Write([Int32]$w)
  $bw.Write([Int32]($h * 2))
  $bw.Write([UInt16]1)
  $bw.Write([UInt16]32)
  $bw.Write([UInt32]0)              # BI_RGB
  $bw.Write([UInt32]($w * $h * 4))
  0..3 | ForEach-Object { $bw.Write([UInt32]0) }

  # Pixels, BGRA, bottom-up.
  $rect = New-Object System.Drawing.Rectangle 0, 0, $w, $h
  $data = $bmp.LockBits($rect, 'ReadOnly', ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb))
  $row = New-Object byte[] ($w * 4)
  try {
    for ($y = $h - 1; $y -ge 0; $y--) {
      [System.Runtime.InteropServices.Marshal]::Copy(
        [IntPtr]::Add($data.Scan0, $y * $data.Stride), $row, 0, $row.Length)
      $bw.Write($row)
    }
  } finally { $bmp.UnlockBits($data) }

  # AND mask: all zero (opaque), rows padded to four bytes.
  $maskRow = [Math]::Floor(($w + 31) / 32) * 4
  $zeros = New-Object byte[] ($maskRow * $h)
  $bw.Write($zeros)

  $bw.Flush()
  $bytes = $ms.ToArray()
  $bw.Dispose(); $ms.Dispose()

  # The leading comma is load-bearing. PowerShell unrolls an array on output,
  # so `return $bytes` emits sixty-five thousand separate bytes, the caller
  # collects them into an Object[], and BinaryWriter.Write then matches neither
  # its byte[] nor its char[] overload and writes nothing at all. The directory
  # still records the right length, so the icon file comes out internally
  # inconsistent and every reader rejects it with a different message.
  #
  # `,$bytes` wraps it in a one-element array, which unrolls back to the array.
  return ,$bytes
}

$frames = @{}
foreach ($s in $sizes) {
  if ($s -eq 256) {
    $ms = New-Object System.IO.MemoryStream
    $marks[$s].Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $frames[$s] = $ms.ToArray()
    $ms.Dispose()
  } else {
    $frames[$s] = Get-Dib $marks[$s]
  }
}

$ms = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $ms
$w.Write([UInt16]0)               # reserved
$w.Write([UInt16]1)               # type: icon
$w.Write([UInt16]$sizes.Count)

$offset = 6 + 16 * $sizes.Count
foreach ($s in $sizes) {
  $w.Write([Byte]$(if ($s -ge 256) { 0 } else { $s }))   # 0 means 256
  $w.Write([Byte]$(if ($s -ge 256) { 0 } else { $s }))
  $w.Write([Byte]0)               # palette entries
  $w.Write([Byte]0)               # reserved
  $w.Write([UInt16]1)             # colour planes
  $w.Write([UInt16]32)            # bits per pixel
  $w.Write([UInt32]$frames[$s].Length)
  $w.Write([UInt32]$offset)
  $offset += $frames[$s].Length
}
foreach ($s in $sizes) { $w.Write([byte[]]$frames[$s]) }

$w.Flush()
$ico = $ms.ToArray()
$w.Dispose(); $ms.Dispose()

# Verify before writing, because the failure this guards against is silent: a
# directory that promises bytes the file does not contain still looks like an
# icon and still has a plausible size.
$want = 6 + 16 * $sizes.Count + ($sizes | ForEach-Object { $frames[$_].Length } | Measure-Object -Sum).Sum
if ($ico.Length -ne $want) {
  throw "icon is $($ico.Length) bytes but its directory describes $want"
}

[System.IO.File]::WriteAllBytes((Join-Path $dir 'icon.ico'), $ico)
foreach ($s in $sizes) { $marks[$s].Dispose() }

Get-ChildItem $dir | Select-Object Name, Length | Format-Table -AutoSize
