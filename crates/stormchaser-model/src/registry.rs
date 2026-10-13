//! Matching container images to the registry a set of credentials belongs to.
//!
//! A container runtime sends pull credentials to whichever registry the image names, so
//! credentials must only travel with an image that lives on their own registry.

const DOCKER_HUB: &str = "docker.io";

fn normalize_host(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    let host = host.strip_suffix(":443").unwrap_or(&host);
    match host {
        "" | "index.docker.io" | "registry-1.docker.io" | "registry.hub.docker.com" => {
            DOCKER_HUB.to_string()
        }
        other => other.to_string(),
    }
}

/// The registry host (with port, if any) an image reference is pulled from.
///
/// Follows the container runtimes' rule: the first path component names a registry only if
/// it contains a `.` or a `:` or is `localhost`; otherwise the image is on Docker Hub.
pub fn image_registry_host(image: &str) -> String {
    match image.split_once('/') {
        Some((first, _)) if first.contains('.') || first.contains(':') || first == "localhost" => {
            normalize_host(first)
        }
        _ => DOCKER_HUB.to_string(),
    }
}

/// The registry host (with port, if any) a registry connection's URL points at. An empty
/// URL means Docker Hub.
pub fn registry_url_host(url: &str) -> String {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = without_scheme.split('/').next().unwrap_or_default();
    // Drop any userinfo: the host is what follows the last `@`.
    let host = authority.rsplit('@').next().unwrap_or_default();
    normalize_host(host)
}

/// True when `image` is pulled from the registry at `registry_url`.
pub fn image_is_on_registry(image: &str, registry_url: &str) -> bool {
    image_registry_host(image) == registry_url_host(registry_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_hosts_follow_the_runtime_rule() {
        assert_eq!(image_registry_host("alpine"), "docker.io");
        assert_eq!(image_registry_host("alpine:3.20"), "docker.io");
        assert_eq!(image_registry_host("library/alpine"), "docker.io");
        assert_eq!(image_registry_host("docker.io/library/alpine"), "docker.io");
        assert_eq!(
            image_registry_host("registry.example.com/team/app:1.2"),
            "registry.example.com"
        );
        assert_eq!(
            image_registry_host("Registry.Example.com:5000/app@sha256:abc"),
            "registry.example.com:5000"
        );
        assert_eq!(image_registry_host("localhost/app"), "localhost");
    }

    #[test]
    fn registry_urls_reduce_to_a_host() {
        assert_eq!(registry_url_host(""), "docker.io");
        assert_eq!(
            registry_url_host("https://index.docker.io/v1/"),
            "docker.io"
        );
        assert_eq!(
            registry_url_host("https://registry.example.com/v2/"),
            "registry.example.com"
        );
        assert_eq!(
            registry_url_host("registry.example.com:443"),
            "registry.example.com"
        );
        assert_eq!(
            registry_url_host("oci://registry.example.com:5000/team"),
            "registry.example.com:5000"
        );
        assert_eq!(
            registry_url_host("https://user@registry.example.com"),
            "registry.example.com"
        );
    }

    #[test]
    fn credentials_only_match_their_own_registry() {
        let url = "https://registry.example.com";
        assert!(image_is_on_registry("registry.example.com/team/app:1", url));
        assert!(!image_is_on_registry("attacker.example.net/app", url));
        assert!(!image_is_on_registry(
            "registry.example.com.attacker.net/app",
            url
        ));
        assert!(!image_is_on_registry("registry.example.com:5000/app", url));
        assert!(!image_is_on_registry("alpine", url));
        assert!(image_is_on_registry("alpine", ""));
        assert!(!image_is_on_registry("registry.example.com/app", ""));
    }
}
