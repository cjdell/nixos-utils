{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.11";
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
      in
      {
        packages.default = list-generations;
        packages.list-generations = list-generations;
        packages.list-containers = list-containers;
      }
    )
    # ==== Modules ====
    // {
      nixosModules.containers = ./modules/containers.nix;
      nixosModules.notifications = ./modules/notifications.nix;
      nixosModules.rollback = ./modules/rollback.nix;
    };
}
