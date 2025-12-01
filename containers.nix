{
  pkgs,
  lib,
  config,
}:

let
  cfg = config.system.updateContainers;

  update-containers = (
    pkgs.writers.writeNuBin "update-containers" (
      pkgs.replaceVars ./nu/update-containers.nu {
        PODMAN = lib.getExe pkgs.podman;
        SYSTEMCTL = "${pkgs.systemd}/bin/systemctl";
        CURL = lib.getExe pkgs.curl;
        WEBHOOK_URL = cfg.webhookUrl;
      }
    )
  );
in
{
  options.system.updateContainers = {
    enable = lib.mkEnableOption "automatic update of containers";

    webhookUrl = lib.mkOption {
      type = lib.types.str;
      default = "";
      example = "https://hooks.slack.com/services/blah/blah/blah";
      description = ''
        URL for webhook explaining what has been updated
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [
      update-containers
    ];

    systemd.timers = {
      update-containers = {
        timerConfig = {
          Unit = "update-containers.service";
          OnCalendar = "*-*-* 02:00:00"; # Run everyday at 2am
          Persistent = true; # Run missed timers on boot
        };
        wantedBy = [ "timers.target" ];
      };
    };

    # sudo systemctl start update-containers.service
    # journalctl -u update-containers.service -f
    systemd.services = {
      update-containers = {
        serviceConfig = {
          Type = "oneshot";
          ExecStart = "${update-containers}/bin/update-containers";
        };
        # Add these for better logging
        after = [ "network-online.target" ];
        wants = [ "network-online.target" ];
      };
    };
  };
}
