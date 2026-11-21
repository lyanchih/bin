use crate::model::nonempty;
use serde::Serialize;

#[derive(Serialize)]
pub struct NetworkMount {
    pub mount_point: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl NetworkMount {
    pub fn new(mount_point: String, filesystem: Option<String>, source: &str) -> Self {
        Self {
            mount_point,
            filesystem,
            source: network_source(source),
        }
    }
}

#[cfg(unix)]
pub fn is_network_filesystem(filesystem: &str) -> bool {
    matches!(
        filesystem,
        "nfs"
            | "nfs4"
            | "cifs"
            | "smb"
            | "smb3"
            | "smbfs"
            | "afpfs"
            | "webdav"
            | "davfs"
            | "afs"
            | "kafs"
            | "ceph"
            | "coda"
            | "ncpfs"
            | "lustre"
            | "glusterfs"
            | "sshfs"
            | "fuse.sshfs"
            | "fuse.glusterfs"
            | "fuse.ceph-fuse"
            | "fuse.davfs"
            | "fuse.s3fs"
            | "fuse.goofys"
            | "fuse.gcsfuse"
            | "fuse.blobfuse"
            | "fuse.curlftpfs"
    )
}

fn network_source(source: &str) -> Option<String> {
    let source = source.trim();
    if matches!(source, "" | "-" | "none") {
        return None;
    }

    // URLs and SMB sources can contain account names or credentials. Keep the
    // remote server and the share/path, and discard URL queries and fragments.
    let authority = |location: &str| {
        let (host, path) = location.split_at(location.find('/').unwrap_or(location.len()));
        let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
        (!host.is_empty()).then(|| format!("{host}{path}"))
    };
    if let Some((scheme, location)) = source.split_once("://") {
        let location = location.split(['?', '#']).next().unwrap_or_default();
        return authority(location).map(|location| format!("{scheme}://{location}"));
    }
    if let Some(location) = source.strip_prefix("//") {
        return authority(location).map(|location| format!("//{location}"));
    }
    if let Some((host, path)) = source.split_once(":/")
        && let Some((_, host)) = host.rsplit_once('@')
    {
        return nonempty(&format!("{host}:/{path}"));
    }
    nonempty(source)
}

#[cfg(test)]
mod tests {
    use super::network_source;

    #[test]
    fn removes_remote_account_names_and_uri_parameters() {
        for (original, expected) in [
            (
                "https://user:secret@nas/team?token=secret#fragment",
                "https://nas/team",
            ),
            ("//user@nas/share", "//nas/share"),
            ("alice@[2001:db8::1]:/data", "[2001:db8::1]:/data"),
            ("nas:/project", "nas:/project"),
            (r"\\nas\share", r"\\nas\share"),
        ] {
            assert_eq!(network_source(original).as_deref(), Some(expected));
        }
    }
}
