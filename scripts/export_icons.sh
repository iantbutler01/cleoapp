#!/bin/bash
# Export Cleo icon to various sizes

# Inkscape CLI location (macOS app bundle)
INKSCAPE="/Applications/Inkscape.app/Contents/MacOS/inkscape"

SVG="/Users/crow/SoftwareProjects/cleoapp/cleo_icon.svg"
SVG_MONO="/Users/crow/SoftwareProjects/cleoapp/cleo_icon_monochrome.svg"
WEB_PUBLIC="/Users/crow/SoftwareProjects/cleoapp/web/public"
DAEMON_ASSETS="/Users/crow/SoftwareProjects/cleoapp/daemon/assets"

mkdir -p "$WEB_PUBLIC"
mkdir -p "$DAEMON_ASSETS"

# Web assets
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$WEB_PUBLIC/apple-touch-icon.png" -w 180 -h 180
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$WEB_PUBLIC/icon-192.png" -w 192 -h 192
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$WEB_PUBLIC/icon-512.png" -w 512 -h 512

# Menu bar icon (template image - monochrome white with alpha)
# Use rsvg-convert for better quality at small sizes (brew install librsvg)
if command -v rsvg-convert &> /dev/null; then
    rsvg-convert -w 22 -h 22 "$SVG_MONO" -o "$DAEMON_ASSETS/menubar-icon.png"
    rsvg-convert -w 44 -h 44 "$SVG_MONO" -o "$DAEMON_ASSETS/menubar-icon@2x.png"
else
    "$INKSCAPE" "$SVG_MONO" --export-type=png --export-filename="$DAEMON_ASSETS/menubar-icon.png" -w 22 -h 22
    "$INKSCAPE" "$SVG_MONO" --export-type=png --export-filename="$DAEMON_ASSETS/menubar-icon@2x.png" -w 44 -h 44
fi

# App icon for macOS - export to iconset folder for iconutil
ICONSET="$DAEMON_ASSETS/AppIcon.iconset"
mkdir -p "$ICONSET"

"$INKSCAPE" "$SVG" --export-type=png --export-filename="$ICONSET/icon_16x16.png" -w 16 -h 16
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$ICONSET/icon_16x16@2x.png" -w 32 -h 32
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$ICONSET/icon_32x32.png" -w 32 -h 32
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$ICONSET/icon_32x32@2x.png" -w 64 -h 64
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$ICONSET/icon_128x128.png" -w 128 -h 128
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$ICONSET/icon_128x128@2x.png" -w 256 -h 256
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$ICONSET/icon_256x256.png" -w 256 -h 256
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$ICONSET/icon_256x256@2x.png" -w 512 -h 512
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$ICONSET/icon_512x512.png" -w 512 -h 512
"$INKSCAPE" "$SVG" --export-type=png --export-filename="$ICONSET/icon_512x512@2x.png" -w 1024 -h 1024

# Generate .icns file from iconset
iconutil -c icns "$ICONSET" -o "$DAEMON_ASSETS/AppIcon.icns"

echo "Done! Icons exported to:"
echo "  $WEB_PUBLIC"
echo "  $DAEMON_ASSETS"
echo "  $DAEMON_ASSETS/AppIcon.icns"
echo ""
echo "Menu bar icons use monochrome template for light/dark mode support."
