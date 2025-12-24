{
  pkgs,
  lib,
  config,
  ...
}:

let
  cfg = config.notifications.gateway;
in
{
  options = {
    notifications.gateway = {
      enable = lib.mkEnableOption "Push Notification Gateway";

      port = lib.mkOption {
        type = lib.types.int;
        default = 8888;
        description = "Listen port";
      };

      notifyUrl = lib.mkOption {
        type = lib.types.str;
        default = null;
        description = "Endpoint for sending notifications";
        example = "http://127.0.0.1:8123/api/services/notify/mobile_app_hd1913";
      };

      payloadFormat = lib.mkOption {
        type = lib.types.enum [
          "home_assistant"
          "slack"
        ];
        default = "home_assisant";
        description = ''
          The payment format the endpoint expects.
          home_assistant = { title: string, message: string }
          slack = { data: string }
        '';
      };

      headerFile = lib.mkOption {
        type = lib.types.str;
        default = null;
        description = ''
          Fully qualified path to the file containing the authorisation header text. Example contents:
          Authorization: Bearer eyJ...
        '';
        example = "/path/to/secret/header";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    # journalctl -u push-notification-gateway -f
    systemd.services.push-notification-gateway = {
      serviceConfig = {
        Type = "simple";
        Restart = "always";
        RestartSec = 5;
        ExecStart = "${lib.getExe pkgs.deno} run --allow-net --allow-env --allow-read ${../deno/notify.ts}";
      };

      environment = {
        PORT = "${toString cfg.port}";
        NOTIFY_URL = "${cfg.notifyUrl}";
        PAYLOAD_FORMAT = "${cfg.payloadFormat}";
        HEADER_FILE = "${cfg.headerFile}";
      };

      after = [ "network-online.target" ];
      requires = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];
    };
  };
}
