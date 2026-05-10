#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TAURI_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${TAURI_DIR}/.." && pwd)"
TARGET_TRIPLE="$(rustc -vV | awk '/host:/ { print $2 }')"
PROFILE="${CTX_SIDECAR_PROFILE:-debug}"

if [[ -z "${TARGET_TRIPLE}" ]]; then
  echo "failed to determine Rust host target triple" >&2
  exit 1
fi

if [[ "${PROFILE}" == "release" ]]; then
  cargo build --manifest-path "${REPO_ROOT}/Cargo.toml" -p ctx-cli --release
  SOURCE_BIN="${REPO_ROOT}/target/release/ctx"
else
  cargo build --manifest-path "${REPO_ROOT}/Cargo.toml" -p ctx-cli
  SOURCE_BIN="${REPO_ROOT}/target/debug/ctx"
fi

if [[ "${OS:-}" == "Windows_NT" ]]; then
  SOURCE_BIN="${SOURCE_BIN}.exe"
fi

mkdir -p "${TAURI_DIR}/bin"
cp "${SOURCE_BIN}" "${TAURI_DIR}/bin/ctx-${TARGET_TRIPLE}"

echo "prepared Tauri sidecar: src-tauri/bin/ctx-${TARGET_TRIPLE}"
