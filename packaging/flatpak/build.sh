#!/bin/sh
# Make a Flatpak of Brolga from programs that are already built:
#
#   packaging/flatpak/build.sh <brolga> <brolga-cli> <out.flatpak>
#
# It needs `flatpak` and the GNOME runtime named below, and nothing else.
# This is the package for installing by hand. Flathub builds from source
# with no network, which wants a manifest of its own.
set -eu

ID=io.github.codemonkey76.Brolga
RUNTIME=org.gnome.Platform
VERSION=50

app=$1
cli=$2
out=$3
here=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The runtime stands in for the SDK: nothing is compiled here.
flatpak build-init "$work/build" "$ID" "$RUNTIME//$VERSION" "$RUNTIME//$VERSION"
files=$work/build/files

install -Dm755 "$app" "$files/bin/brolga"
install -Dm755 "$cli" "$files/bin/brolga-cli"
for size in 32 128; do
    install -Dm644 "$here/app/src-tauri/icons/${size}x${size}.png" \
        "$files/share/icons/hicolor/${size}x${size}/apps/$ID.png"
done
install -Dm644 "$here/app/src-tauri/icons/128x128@2x.png" \
    "$files/share/icons/hicolor/256x256/apps/$ID.png"
install -Dm644 "$here/app/src-tauri/icons/icon.png" \
    "$files/share/icons/hicolor/512x512/apps/$ID.png"

# The system tray is drawn by libraries the runtime does not have. They are
# taken from this computer where it has them; without them Brolga has no
# tray icon and closing its window quits it.
for lib in libayatana-appindicator3.so.1 libayatana-indicator3.so.7 libayatana-ido3-0.4.so.0 \
    libdbusmenu-gtk3.so.4 libdbusmenu-glib.so.4; do
    for dir in /usr/lib/x86_64-linux-gnu /usr/lib64 /usr/lib; do
        if [ -e "$dir/$lib" ]; then
            install -Dm755 "$(readlink -f "$dir/$lib")" "$files/lib/$lib"
            break
        fi
    done
done

mkdir -p "$files/share/applications" "$files/share/metainfo"
cat > "$files/share/applications/$ID.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Brolga
GenericName=Mail and Calendar
Comment=Mail, calendar and contacts for Microsoft 365
Exec=brolga %u
Icon=$ID
Terminal=false
Categories=Network;Email;Office;Calendar;ContactManagement;
Keywords=mail;email;e-mail;calendar;contacts;
MimeType=x-scheme-handler/mailto;
StartupNotify=true
StartupWMClass=brolga
DESKTOP
sed "s#<launchable type=\"desktop-id\">Brolga.desktop</launchable>#<launchable type=\"desktop-id\">$ID.desktop</launchable>#" \
    "$here/packaging/linux/$ID.metainfo.xml" > "$files/share/metainfo/$ID.metainfo.xml"

# What the app may reach, and no more:
#   the network, for Microsoft 365 and for sign-in's answer on localhost;
#   the window, on Wayland or X11, drawn with the graphics card;
#   the secret store, where the sign-in is kept;
#   notifications, and the system tray where there is one;
#   the Downloads folder, where attachments are saved.
# Any other file is reached only through the system's own file dialog.
flatpak build-finish "$work/build" \
    --command=brolga \
    --share=network \
    --share=ipc \
    --socket=wayland \
    --socket=fallback-x11 \
    --device=dri \
    --talk-name=org.freedesktop.secrets \
    --talk-name=org.freedesktop.Notifications \
    --talk-name=org.kde.StatusNotifierWatcher \
    --filesystem=xdg-download

flatpak build-export "$work/repo" "$work/build" >/dev/null
flatpak build-bundle "$work/repo" "$out" "$ID" \
    --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
echo "Made $out"
