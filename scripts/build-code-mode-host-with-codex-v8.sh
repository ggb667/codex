#!/usr/bin/env bash
# Build codex-code-mode-host with Codex-published rusty_v8 artifacts.
#
# This avoids two local Cargo traps for v8 v150.4.0:
# - upstream denoland rusty_v8 sandbox prebuilts may be absent for this target;
# - V8_FROM_SOURCE needs Chromium vendored inputs that are not present in the crate source tree.

set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/build-code-mode-host-with-codex-v8.sh [OPTIONS]

Options:
  --target <triple>       Rust target triple. Defaults to `rustc -vV` host.
  --target-dir <dir>      Cargo target dir. Defaults to /tmp/codex-code-host-codex-v8-target.
  --artifact-dir <dir>    Download/cache dir for rusty_v8 artifacts. Defaults under /tmp.
  --cargo-home <dir>      CARGO_HOME for the build. Defaults to existing CARGO_HOME or /tmp/cargo-home.
  --no-deploy             Build only; do not copy into codex-rs/target/debug.
  --no-restore-lock       Leave any Cargo.lock update in place.
  -h, --help              Show this help.

Environment:
  CARGO_BUILD_JOBS        Defaults to 1 if unset.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

host_triple="$(rustc -vV | awk '/^host: / { print $2 }')"
target="${host_triple}"
target_dir="/tmp/codex-code-host-codex-v8-target"
artifact_dir=""
cargo_home="${CARGO_HOME:-/tmp/cargo-home}"
deploy=1
restore_lock=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      target="${2:?missing value for --target}"
      shift 2
      ;;
    --target-dir)
      target_dir="${2:?missing value for --target-dir}"
      shift 2
      ;;
    --artifact-dir)
      artifact_dir="${2:?missing value for --artifact-dir}"
      shift 2
      ;;
    --cargo-home)
      cargo_home="${2:?missing value for --cargo-home}"
      shift 2
      ;;
    --no-deploy)
      deploy=0
      shift
      ;;
    --no-restore-lock)
      restore_lock=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

version="$(python3 .github/scripts/rusty_v8_bazel.py resolved-v8-crate-version)"
profile="ptrcomp_sandbox_release"
release_tag="rusty-v8-v${version}"
base_url="https://github.com/openai/codex/releases/download/${release_tag}"
artifact_dir="${artifact_dir:-/tmp/rusty_v8_codex_${version}_${target}}"

if [[ "${target}" == *-pc-windows-msvc ]]; then
  archive_name="rusty_v8_${profile}_${target}.lib.gz"
else
  archive_name="librusty_v8_${profile}_${target}.a.gz"
fi
binding_name="src_binding_${profile}_${target}.rs"
checksums_name="rusty_v8_${profile}_${target}.sha256"
archive_path="${artifact_dir}/${archive_name}"
binding_path="${artifact_dir}/${binding_name}"
checksums_path="${artifact_dir}/${checksums_name}"
trusted_checksums="third_party/v8/rusty_v8_${version//./_}_release_manifests.sha256"

if command -v sha256sum >/dev/null 2>&1; then
  checksum_command=(sha256sum --check -)
else
  checksum_command=(shasum -a 256 --check -)
fi

mkdir -p "${artifact_dir}" "${cargo_home}"

echo "Using rusty_v8 ${version} for ${target} from ${base_url}"

if [[ ! -s "${checksums_path}" ]]; then
  curl -fsSL "${base_url}/${checksums_name}" -o "${checksums_path}"
fi

expected_manifest_checksum="$(grep -F "  ${checksums_name}" "${trusted_checksums}" | cut -d ' ' -f 1)"
actual_manifest_checksum="$(python3 -c 'import hashlib, pathlib, sys; print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())' "${checksums_path}")"
if [[ -z "${expected_manifest_checksum}" || "${actual_manifest_checksum}" != "${expected_manifest_checksum}" ]]; then
  echo "Checksum mismatch for ${checksums_name}: expected ${expected_manifest_checksum:-<missing>}, got ${actual_manifest_checksum}" >&2
  exit 1
fi

if [[ ! -s "${archive_path}" ]]; then
  curl -fL "${base_url}/${archive_name}" -o "${archive_path}"
fi
if [[ ! -s "${binding_path}" ]]; then
  curl -fsSL "${base_url}/${binding_name}" -o "${binding_path}"
fi

(
  cd "${artifact_dir}"
  tr -d '\r' < "${checksums_path}" | "${checksum_command[@]}"
)

lock_was_dirty=0
if ! git -C codex-rs diff --quiet -- Cargo.lock; then
  lock_was_dirty=1
fi

cargo_args=(build -p codex-code-mode-host --bin codex-code-mode-host)
if [[ "${target}" != "${host_triple}" ]]; then
  cargo_args+=(--target "${target}")
fi

(
  cd codex-rs
  CARGO_HOME="${cargo_home}" \
  CARGO_TARGET_DIR="${target_dir}" \
  CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}" \
  RUSTY_V8_ARCHIVE="${archive_path}" \
  RUSTY_V8_SRC_BINDING_PATH="${binding_path}" \
  cargo "${cargo_args[@]}"
)

binary_name="codex-code-mode-host"
if [[ "${target}" == *windows* ]]; then
  binary_name="codex-code-mode-host.exe"
fi
if [[ "${target}" == "${host_triple}" ]]; then
  built_binary="${target_dir}/debug/${binary_name}"
else
  built_binary="${target_dir}/${target}/debug/${binary_name}"
fi
if [[ ! -x "${built_binary}" ]]; then
  echo "Expected built binary not found: ${built_binary}" >&2
  exit 1
fi

if [[ "${restore_lock}" -eq 1 && "${lock_was_dirty}" -eq 0 ]] && ! git -C codex-rs diff --quiet -- Cargo.lock; then
  git -C codex-rs checkout -- Cargo.lock
fi

if [[ "${deploy}" -eq 1 ]]; then
  install_dir="codex-rs/target/debug"
  install_path="${install_dir}/${binary_name}"
  mkdir -p "${install_dir}"
  if [[ -e "${install_path}" ]]; then
    cp -a "${install_path}" "${install_path}.pre-code-host-deploy.$(date +%Y%m%d%H%M%S)"
  fi
  cp -a "${built_binary}" "${install_path}"
  chmod +x "${install_path}"
  echo "Deployed ${install_path}"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${install_path}"
  else
    shasum -a 256 "${install_path}"
  fi
else
  echo "Built ${built_binary}"
fi
