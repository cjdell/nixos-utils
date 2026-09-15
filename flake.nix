{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-26.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      ...
    }:
    # ==== Packages ====
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        list-generations = (pkgs.writers.writeNuBin "list-generations" ./nu/list-generations.nu);
        list-containers = (pkgs.writers.writeNuBin "list-containers" ./nu/list-containers.nu);
        container-ui = pkgs.rustPlatform.buildRustPackage {
          pname = "container-ui";
          version = "0.1.0";
          src = ./container-ui;
          cargoLock = {
            lockFile = ./container-ui/Cargo.lock;
          };
          meta.description = "Server-rendered web UI for managing podman containers";
        };
        health-backend = pkgs.callPackage ./health/backend/package.nix { };
        health-web = pkgs.callPackage ./health/web/package.nix { };
        health = pkgs.callPackage ./health/package.nix {
          backend = health-backend;
          web = health-web;
        };
      in
      {
        packages.default = list-generations;
        packages.list-generations = list-generations;
        packages.list-containers = list-containers;
        packages.container-ui = container-ui;
        packages.health-backend = health-backend;
        packages.health-web = health-web;
        packages.health = health;
      }
    )
    # ==== Modules ====
    // {
      nixosModules.containers = ./modules/containers.nix;
      nixosModules.notifications = ./modules/notifications.nix;
      nixosModules.rollback = ./modules/rollback.nix;
      nixosModules.health = ./modules/health.nix;
    };
}
