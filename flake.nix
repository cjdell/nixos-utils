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
      in
      {
        packages.default = list-generations;
        packages.list-generations = list-generations;
      }
    )
    # ==== Modules ====
    // {
      modules.rollback = ./rollback.nix;
      modules.containers = ./containers.nix;
    };
}
