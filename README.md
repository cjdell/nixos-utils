# NixOS Utils

A collection of NixOS modules that I use in my home lab.

## Modules

### Container Updates

Automatically upgrade containers on interval

#### Tools

```bash
sudo update-containers
sudo list-containers
```

#### Module

```nix
modules = [
  ...
  nixos-utils.nixosModules.containers
  ...
];
```

#### Configuration

```nix
system.updateContainers = {
  enable = true;
  webhookUrl = "http://localhost:8888";
};
```

### Auto Rollbacks

Automatically rollback to previous generation if an update isn't confirmed by the user.

#### Tools

```bash
list-generations    # List generations include "good", "current", and "booted" 
sudo nixos-confirm  # Set this generation as "good" and prevent auto-rollback
```

#### Module

```nix
modules = [
  ...
  nixos-utils.nixosModules.rollback
  ...
];
```

#### Configuration

```nix
system.autoRollback.enable = true;
```

### Push Notification Gateway

A gateway to either Home Assistant or Slack with secret header specified as a file for proper secrets management.

#### Module

```nix
modules = [
  ...
  nixos-utils.nixosModules.notifications
  ...
];
```

#### Configuration

```nix
notifications.gateway = {
  enable = true;
  port = 8888;
  notifyUrl = "http://192.168.49.1:8123/api/services/notify/mobile_app_hd1913";
  payloadFormat = "home_assistant";
  # Just for testing. Use `sops-nix` in production for proper secrets management
  headerFile = "${pkgs.writeText "header-file" "Authorization: Bearer eyJ...hKs"}"; 
};
```

```bash
curl -X POST http://localhost:8888 -H 'Content-Type: application/json' -d '{"message":"Hello World!","title":"Notification Test"}'
```
