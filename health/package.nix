{
  pkgs,
  lib,
  backend,
  web,
  ...
}:

# Combined deployment artifact: the backend binary plus the SPA static files,
# laid out so a service can run `${pkg}/bin/health-backend` with
# `STATIC_DIR=${pkg}/share/health`.

pkgs.stdenv.mkDerivation {
  pname = "health";
  version = "0.1.0";
  src = ./.;
  buildPhase = "true";
  installPhase = ''
    mkdir -p $out/bin $out/share/health
    cp ${backend}/bin/health-backend $out/bin/health-backend
    cp -r ${web}/* $out/share/health/
  '';
  meta = {
    description = "Health monitoring dashboard (backend + SPA)";
    license = lib.licenses.mit;
  };
  passthru = {
    inherit backend web;
  };
}
