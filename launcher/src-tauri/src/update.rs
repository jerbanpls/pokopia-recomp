// SPDX-License-Identifier: GPL-3.0-or-later
//! "Update available" check. One GET to the GitHub releases API at start, never any download or install: the launcher only
//! tells the player that a newer release exists and links to its page. Every failure is silent (one line on stderr).

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::io::Read;
use std::time::Duration;

/// The Linux edition's releases, tagged `v<upstream version>-linux.<n>` (v0.5.0-linux.1).
const REPO: &str = "jerbanpls/pokopia-recomp";
const TIMEOUT: Duration = Duration::from_secs(5);
/// Builds from a checkout carry these (0.0.0 = untagged build, 0.1.1 = the Cargo.toml default); a tagged release stamps its own.
const DEV_VERSIONS: [&str; 2] = ["0.0.0", "0.1.1"];

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct UpdateInfo {
    /// the tag as published, e.g. "v0.3.0"
    pub tag: String,
    pub name: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// A parsed `major.minor.patch[-pre]` version (a leading `v` and `+build` metadata are ignored).
#[derive(Debug, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    pre: Vec<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix('v').or_else(|| s.strip_prefix('V')).unwrap_or(s);
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let mut nums = core.split('.');
        let mut core3 = [0u64; 3];
        for slot in core3.iter_mut() {
            let part = nums.next()?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            *slot = part.parse().ok()?;
        }
        if nums.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(p) => {
                let ids: Vec<String> = p.split('.').map(str::to_string).collect();
                if ids.iter().any(|i| i.is_empty() || !i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
                    return None;
                }
                ids
            }
        };
        Some(Version { core: core3, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater, // a release is newer than its own prerelease
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    let o = match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => Ordering::Less, // numeric identifiers rank below alphanumeric ones
                        (Err(_), Ok(_)) => Ordering::Greater,
                        _ => a.cmp(b),
                    };
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        })
    }
}
impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// True when `tag` is a valid version newer than `current`. Anything unparsable is "not newer".
pub fn is_newer(tag: &str, current: &str) -> bool {
    match (Version::parse(tag), Version::parse(current)) {
        (Some(t), Some(c)) => t > c,
        _ => false,
    }
}

pub fn is_dev_version(v: &str) -> bool {
    DEV_VERSIONS.contains(&v.trim())
}


fn get(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let resp = agent.get(url).set("Accept", "application/vnd.github+json").call().map_err(|e| e.to_string())?;
    let mut body = Vec::new();
    resp.into_reader().take(2 << 20).read_to_end(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}

fn fetch(current: &str) -> Result<Option<UpdateInfo>, String> {
    let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).user_agent(&format!("PokopiaRecomp-Launcher/{}", current)).build();
    // /releases/latest never returns drafts or prereleases
    let body = get(&agent, &format!("https://api.github.com/repos/{}/releases/latest", REPO))?;
    let r: Release = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    if r.draft || r.prerelease || !is_newer(&r.tag_name, current) || !r.html_url.starts_with("https://github.com/") {
        return Ok(None);
    }
    Ok(Some(UpdateInfo {
        name: r.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| r.tag_name.clone()),
        tag: r.tag_name,
        url: r.html_url,
    }))
}

/// None for "no update" and for every error (one line on stderr).
pub fn check(current: &str) -> Option<UpdateInfo> {
    match fetch(current) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("update check skipped: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod linux_tests {
    use super::*;

    #[test]
    fn linux_edition_tags() {
        assert!(is_newer("v0.5.0-linux.2", "0.5.0-linux.1"));
        assert!(is_newer("v0.5.0-linux.10", "0.5.0-linux.9"));
        assert!(is_newer("v0.5.1-linux.1", "0.5.0-linux.3"));
        assert!(!is_newer("v0.5.0-linux.1", "0.5.0-linux.1"));
        assert!(!is_dev_version("0.5.0-linux.1"));
    }
}
