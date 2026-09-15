{
  pkgs,
  lib,
  ...
}:

pkgs.rustPlatform.buildRustPackage {
  pname = "health-backend";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;

  meta = {
    description = "System health metrics HTTP API + SPA server";
    mainProgram = "health-backend";
    license = lib.licenses.mit;
  };
}
