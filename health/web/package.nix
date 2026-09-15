{
  pkgs,
  lib,
  ...
}:

# Builds the dioxus SPA to wasm and bundles it with index.html + style.css.
#
# The toolchain dance, in short:
# - `cargo build --target wasm32-unknown-unknown` (nixpkgs rustc ships the
#   wasm32 std; `lld` provides the `wasm-ld` linker)
# - `wasm-bindgen --target web` turns the .wasm into a loadable ES module.
#   The CLI in nixpkgs is 0.2.121, so Cargo.toml pins wasm-bindgen/js-sys/
#   web-sys/wasm-bindgen-futures to the matching schema versions.
# - `wasm-opt` shrinks the binary (needs bulk-memory + nontrapping-fptoint
#   flags, which dioxus/rust emit these days).

let
  src = lib.cleanSourceWith {
    src = ./.;
    filter =
      path: type:
      let
        base = baseNameOf path;
      in
      !(type == "directory" && (base == "target" || base == "dist"));
  };
in
pkgs.stdenv.mkDerivation {
  pname = "health-web";
  version = "0.1.0";
  inherit src;

  cargoDeps = pkgs.rustPlatform.importCargoLock {
    lockFile = ./Cargo.lock;
  };

  nativeBuildInputs = [
    pkgs.rustPlatform.cargoSetupHook
    pkgs.rustc
    pkgs.cargo
    pkgs.wasm-bindgen-cli
    pkgs.binaryen
    pkgs.lld
  ];

  # cargo wants a writable HOME
  env.HOME = "/tmp";

  buildPhase = ''
    runHook preBuild
    cargo build --release --target wasm32-unknown-unknown
    mkdir -p dist
    wasm-bindgen --target web --out-dir dist --no-typescript \
      target/wasm32-unknown-unknown/release/health-ui.wasm
    wasm-opt --enable-bulk-memory --enable-nontrapping-float-to-int -O2 \
      -o dist/health-ui_bg.wasm dist/health-ui_bg.wasm
    cp index.html style.css dist/
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -r dist/* $out/
    runHook postInstall
  '';

  meta = {
    description = "Health dashboard SPA (dioxus, compiled to wasm)";
    license = lib.licenses.mit;
  };
}
