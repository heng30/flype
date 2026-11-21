# 图标

源图是仓库根的 `brand.png`，各尺寸 PNG 与 `hicolor/scalable/apps/flype.svg` 都由它生成（换了源图后重新跑）：

```bash
for s in 16 22 24 32 48 64 128 256; do
  magick brand.png -filter Lanczos -resize ${s}x${s} -strip \
    icons/hicolor/${s}x${s}/apps/flype.png
done
```

`flype.svg` 只是内嵌了 512px 位图的壳，给偏爱 scalable 目录的桌面用：

```bash
magick brand.png -filter Lanczos -resize 512x512 -strip /tmp/flype-512.png
b64=$(base64 -w0 /tmp/flype-512.png)
cat > icons/hicolor/scalable/apps/flype.svg <<EOF
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="512" height="512" viewBox="0 0 512 512">
  <title>Flype</title>
  <image width="512" height="512" xlink:href="data:image/png;base64,$b64"/>
</svg>
EOF
```

图标名 `flype`：fcitx5 输入法列表、托盘和「Flype 设置」都用它。
