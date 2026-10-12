#!/usr/bin/env bash
# Writes the update manifest for a release from the packages in the current
# folder: `update-manifest.sh v0.1.6 https://…/releases/download/v0.1.6`.
#
# Each package that has a signature beside it (`name.sig`) is listed under
# the kind of install it replaces. One with no signature is left out, and a
# copy installed that way is pointed to the release page instead.
set -euo pipefail

tag="$1"
base="$2"
platforms='{}'

# list <key> <file>: add the file under the key, if it and its signature are here.
list() {
    local key="$1" file="$2"
    if [ -n "$file" ] && [ -f "$file" ] && [ -f "$file.sig" ]; then
        platforms=$(jq --arg key "$key" --arg url "$base/$file" --arg signature "$(cat "$file.sig")" \
            '. + {($key): {url: $url, signature: $signature}}' <<<"$platforms")
    fi
}

first() { ls $1 2>/dev/null | head -n 1 || true; }

appimage=$(first '*.AppImage')
installer=$(first '*-setup.exe')
mac=$(first '*.app.tar.gz')

list linux-x86_64 "$appimage"
list linux-x86_64-appimage "$appimage"
list linux-x86_64-deb "$(first '*.deb')"
list linux-x86_64-rpm "$(first '*.rpm')"
list windows-x86_64 "$installer"
list windows-x86_64-nsis "$installer"
list windows-x86_64-msi "$(first '*.msi')"
# One package for both kinds of Mac.
list darwin-aarch64 "$mac"
list darwin-x86_64 "$mac"

if [ "$platforms" = '{}' ]; then
    echo "No package here has a signature, so there is nothing an installed Brolga could update to." >&2
    exit 1
fi

jq -n --arg version "${tag#v}" --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --argjson platforms "$platforms" \
    '{version: $version, pub_date: $date, platforms: $platforms}'
