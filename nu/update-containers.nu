#!/usr/bin/env nu

# Test with
# sudo podman tag 35f50ba5c110 docker.io/library/eclipse-mosquitto:latest

## Uncomment to test script
# let PODMAN = "podman"
# let SYSTEMCTL = "systemctl"
# let CURL = "curl"
# let WEBHOOK_URL = "http://127.0.0.1:8888"

let PODMAN = "@PODMAN@"
let SYSTEMCTL = "@SYSTEMCTL@"
let CURL = "@CURL@"
let WEBHOOK_URL = "@WEBHOOK_URL@"

let containers: list<record> = ^$PODMAN ps --format=json | from json | select Names.0 Image Status

mut notification_list: list<string> = []

for container in $containers {
    let old_digest = get_digest $container.Image

    print $"(ansi bo)Updating ($container.Image) - ($old_digest)(ansi reset)"

    ^$PODMAN pull $container.Image # err> /dev/null

    let new_digest = get_digest $container.Image

    if $old_digest != $new_digest {
        ^$SYSTEMCTL restart $"podman-($container.'Names.0')"

        print $"(ansi bo)Updated ($old_digest) => ($new_digest)(ansi reset)"

        $notification_list = $notification_list | append $"🔄 Updated \"($container.Image)\" - ($old_digest) => ($new_digest)";
    } else {
        print $"(ansi bo)Unchanged(ansi reset)"
    }

    print ""
}

print $notification_list

if ((($notification_list | length) > 0) and (($WEBHOOK_URL | str length) > 0)) {
    let data = { title: "Updated Containers", message: ($notification_list | str join "\n") } | to json

    ^$CURL -X POST -H 'Content-type: application/json' --data $data $WEBHOOK_URL
}

def get_digest [image: string] {
  ^$PODMAN inspect $image | from json | get 0.Digest | str replace "sha256:" "" | str substring 0..12
}
