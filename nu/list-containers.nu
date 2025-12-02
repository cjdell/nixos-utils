#!/usr/bin/env nu

podman ps -q | from tsv -n | get "column0" | each {
    let container_id = $in

    let json = (podman inspect $container_id | from json)
    let networks = $json.0.NetworkSettings.Networks

    if ("podman" in $networks) {
        let ip = ($networks.podman.IPAddress | default "N/A")
        let name = ($json.0.Name | str replace "^/" "")

        { Name: $name, IP: $ip, ID: $container_id }
    }
}
