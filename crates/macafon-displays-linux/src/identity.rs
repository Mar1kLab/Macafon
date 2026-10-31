use macafon_ipc::{DisplayIdentity, DisplayMonitor};

pub(crate) fn edid(bytes: &[u8], path: &str) -> Option<(DisplayIdentity, Option<String>)> {
    let base = bytes.get(..128)?;
    if base[..8] != [0, 255, 255, 255, 255, 255, 255, 0]
        || base.iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0
    {
        return None;
    }
    let vendor = u16::from_be_bytes([base[8], base[9]]);
    let manufacturer: String = [10, 5, 0]
        .into_iter()
        .map(|shift| {
            let letter = ((vendor >> shift) & 31) as u8;
            if (1..=26).contains(&letter) {
                char::from(b'A' + letter - 1)
            } else {
                '?'
            }
        })
        .collect();
    let product = u16::from_le_bytes([base[10], base[11]]);
    let number = u32::from_le_bytes([base[12], base[13], base[14], base[15]]);
    let mut serial = (number != 0).then(|| number.to_string());
    let mut name = None;
    for descriptor in base[54..126].chunks_exact(18) {
        if descriptor[..3] == [0, 0, 0] {
            let text = String::from_utf8_lossy(&descriptor[5..18])
                .trim_matches(['\0', '\n', '\r', ' '])
                .to_owned();
            if !text.is_empty() {
                match descriptor[3] {
                    0xff => serial = Some(text),
                    0xfc => name = Some(text),
                    _ => {}
                }
            }
        }
    }
    Some((
        DisplayIdentity {
            edid: format!("{manufacturer}{product:04X}"),
            serial,
            path: path.into(),
        },
        name,
    ))
}
fn connector(name: &str) -> String {
    name.replace("HDMI-A-", "HDMI-").replace("HDMI-B-", "HDMI-")
}

/// Match only an unambiguous DRM connector. Multiple GPUs may both have a DP-1.
pub(crate) fn enrich(monitors: &mut [DisplayMonitor]) {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return;
    };
    let entries: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let (_, port) = name.split_once('-')?;
            let bytes = std::fs::read(entry.path().join("edid")).ok()?;
            let identity = edid(&bytes, port)?;
            Some((connector(port), identity))
        })
        .collect();
    for monitor in monitors {
        let matches: Vec<_> = entries
            .iter()
            .filter(|(port, _)| *port == connector(&monitor.identity.path))
            .collect();
        if let [(_, (identity, name))] = matches.as_slice() {
            monitor.identity.edid = identity.edid.clone();
            if identity.serial.is_some() {
                monitor.identity.serial = identity.serial.clone();
            }
            if let Some(name) = name {
                monitor.name = name.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_edid_and_rejects_invalid_base_blocks() {
        let mut bytes = [0u8; 128];
        bytes[..8].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 0]);
        bytes[8..10].copy_from_slice(&0x1e6du16.to_be_bytes()); // GSM
        bytes[10..12].copy_from_slice(&0x5b7fu16.to_le_bytes());
        bytes[12..16].copy_from_slice(&42u32.to_le_bytes());
        bytes[57] = 0xfc;
        bytes[59..72].copy_from_slice(b"LG ULTRAGEAR\n");
        bytes[127] = 0u8.wrapping_sub(bytes[..127].iter().fold(0u8, |a, b| a.wrapping_add(*b)));
        let (id, name) = edid(&bytes, "HDMI-A-1").expect("EDID");
        assert_eq!(id.edid, "GSM5B7F");
        assert_eq!(id.serial.as_deref(), Some("42"));
        assert_eq!(name.as_deref(), Some("LG ULTRAGEAR"));
        bytes[20] ^= 1;
        assert!(edid(&bytes, "DP-1").is_none());
        assert!(edid(&bytes[..80], "DP-1").is_none());
    }
}
