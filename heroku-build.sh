#!/usr/bin/env bash
# Compile le site (serveur + WebAssembly). Lancé sur Heroku par RustConfig une fois que
# le buildpack emk/rust a installé Rust ; marche aussi à la main, depuis le dépôt :
#   ./heroku-build.sh
# Le Procfile démarre ensuite site/target/release/stratyx-site.
set -euo pipefail

cd "$(dirname "$0")/site"
# Sur Heroku, Cargo compile dans le cache du buildpack (CARGO_TARGET_DIR) : il est gardé
# d'un déploiement à l'autre, les suivants sont bien plus rapides.
target="${CARGO_TARGET_DIR:-target}"

echo "-----> Compilation du serveur"
cargo build --locked --release --bin stratyx-site
mkdir -p target/release
if [ "$target" != target ]; then
  cp "$target/release/stratyx-site" target/release/
fi
echo "       site/target/release/stratyx-site prêt"

echo "-----> Compilation du bundle WebAssembly (îlots, 3D, moteur poker)"
rustup target add wasm32-unknown-unknown
cargo build --locked --lib --target wasm32-unknown-unknown --profile wasm-release

# wasm-bindgen doit avoir la version de site/Cargo.lock : binaire précompilé, ou compilé
# s'il ne peut pas être téléchargé.
version=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')
bin="$target/wasm-bindgen-$version"
if [ ! -x "$bin/wasm-bindgen" ]; then
  mkdir -p "$bin"
  curl -sSfL "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${version}/wasm-bindgen-${version}-x86_64-unknown-linux-musl.tar.gz" \
    | tar -xz --strip-components=1 -C "$bin" \
    || cargo install wasm-bindgen-cli --version "$version" --locked --root "$bin" --bin wasm-bindgen
  [ -x "$bin/wasm-bindgen" ] || mv "$bin/bin/wasm-bindgen" "$bin/"
fi
"$bin/wasm-bindgen" --target web --no-typescript --out-dir pkg \
  "$target/wasm32-unknown-unknown/wasm-release/stratyx_site.wasm"
echo "       bundle prêt dans site/pkg"
