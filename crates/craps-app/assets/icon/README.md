# Application icon

`icon.svg` is the source of truth — the "Seven-Out Curve": the app's
ending-bankroll histogram with its ruin-red bust bar, and a thrown 4-and-3
(seven out) landing at its foot, in the Lamplight palette.

Derived assets (regenerate after editing the SVG; requires `librsvg`
(`brew install librsvg`), macOS `iconutil`, and Python with Pillow):

```sh
cd crates/craps-app/assets/icon
mkdir AppIcon.iconset
for s in 16 32 64 128 256 512 1024; do
  rsvg-convert -w $s -h $s icon.svg -o AppIcon.iconset/tmp-$s.png
done
cd AppIcon.iconset
cp tmp-16.png icon_16x16.png;    cp tmp-32.png  icon_16x16@2x.png
cp tmp-32.png icon_32x32.png;    cp tmp-64.png  icon_32x32@2x.png
cp tmp-128.png icon_128x128.png; cp tmp-256.png icon_128x128@2x.png
cp tmp-256.png icon_256x256.png; cp tmp-512.png icon_256x256@2x.png
cp tmp-512.png icon_512x512.png; cp tmp-1024.png icon_512x512@2x.png
rm tmp-*.png; cd ..
iconutil -c icns AppIcon.iconset -o AppIcon.icns && rm -r AppIcon.iconset
rsvg-convert -w 1024 -h 1024 icon.svg -o /tmp/icon-1024.png
python3 -c "
from PIL import Image
im = Image.open('/tmp/icon-1024.png').convert('RGBA')
im.resize((256,256), Image.LANCZOS).save('icon.ico',
    sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
im.resize((256,256), Image.LANCZOS).save('icon-256.png')"
```

Consumers: `AppIcon.icns` → the macOS bundle (release.yml, CFBundleIconFile);
`icon.ico` → the Windows executable resource (build.rs via winresource);
`icon-256.png` → the runtime window/taskbar icon (main.rs `app_icon()`).
