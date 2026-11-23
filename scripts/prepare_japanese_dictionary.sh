#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 || ! -d "$1" ]]; then
    printf 'Usage: %s <model-directory>\n' "$0" >&2
    exit 2
fi

model_dir="$(cd "$1" && pwd)"
dictionary="$model_dir/g2p_ja"
if [[ -f "$dictionary/metadata.json" ]]; then
    printf 'Japanese dictionary: %s\n' "$dictionary"
    exit 0
fi

work="$model_dir/.g2p_ja.install"
mkdir "$work"
trap 'rm -rf -- "$work"' EXIT
if [[ -e "$dictionary" || -L "$dictionary" ]]; then
    if [[ -f "$dictionary/metadata.json" ]]; then
        printf 'Japanese dictionary: %s\n' "$dictionary"
        exit 0
    fi
    printf 'Dictionary path already exists: %s\n' "$dictionary" >&2
    exit 1
fi

archive="$work/naist-jdic-jpreprocess.tar.gz"
url='https://github.com/jpreprocess/jpreprocess/releases/download/v0.15.0/naist-jdic-jpreprocess.tar.gz'
expected='8a930bbc57bf4adcf521d53544c7dc9ab8ab3aa997a591b1b1608dc5539017b8'
curl -fsSL --retry 2 -o "$archive" "$url"
if command -v sha256sum >/dev/null 2>&1; then
    checksum="$(sha256sum "$archive")"
else
    checksum="$(shasum -a 256 "$archive")"
fi
if [[ "${checksum%% *}" != "$expected" ]]; then
    printf 'Japanese dictionary archive checksum mismatch\n' >&2
    exit 1
fi

tar -xzf "$archive" -C "$work"
mv "$work/naist-jdic" "$dictionary"
printf 'Japanese dictionary: %s\n' "$dictionary"
