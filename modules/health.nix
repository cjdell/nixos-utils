{
  config,
  lib,
  pkgs,
  ...
}:

# NixOS module for the health dashboard.
#
# Usage:
#   services.health.enable = true;
#   # optionally bind elsewhere / tweak retention
#   services.health.listenAddr = "127.0.0.1:8092";
#
# This module only runs the HTTP server; add your own nginx virtual host to
# expose it (see the module in nixos-config's `hosts/grafton-router/` for an
# SSO-protected example).

let
  cfg = config.services.health;

  backend = pkgs.callPackage ../health/backend/package.nix { };
  web = pkgs.callPackage ../health/web/package.nix { };
  health = pkgs.callPackage ../health/package.nix {
    inherit backend web;
  };
in
{
  options.services.health = {
    enable = lib.mkEnableOption "the health monitoring dashboard";

    listenAddr = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:8092";
      description = "Address the HTTP server binds to (nginx should proxy this).";
    };

    sampleIntervalMs = lib.mkOption {
      type = lib.types.int;
      default = 1000;
      description = "How often to sample the system (ms).";
    };

    fastRetentionSecs = lib.mkOption {
      type = lib.types.int;
      default = 3600;
      description = "1s-resolution history kept in memory (seconds).";
    };

    slowIntervalSecs = lib.mkOption {
      type = lib.types.int;
      default = 10;
      description = "Sample every N seconds for the long-range ring buffer.";
    };

    slowRetentionSecs = lib.mkOption {
      type = lib.types.int;
      default = 86400;
      description = "Long-range history kept in memory (seconds).";
    };

    package = lib.mkOption {
      type = lib.types.package;
      default = health;
      description = "The health package to run (backend + static files).";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.health = {
      description = "Health monitoring dashboard";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/health-backend";
        Environment = [
          "LISTEN_ADDR=${cfg.listenAddr}"
          "STATIC_DIR=${cfg.package}/share/health"
          "SAMPLE_INTERVAL_MS=${toString cfg.sampleIntervalMs}"
          "FAST_RETENTION_SECS=${toString cfg.fastRetentionSecs}"
          "SLOW_INTERVAL_SECS=${toString cfg.slowIntervalSecs}"
          "SLOW_RETENTION_SECS=${toString cfg.slowRetentionSecs}"
          "RUST_LOG=info"
        ];
        Restart = "on-failure";
        RestartSec = 3;
      };
    };
  };
}
