//! Windows display profiles over CCD. Only live queries contain adapter/source/target ids.
mod logic;
mod watcher;
pub use logic::{matches, suggestions};
pub use watcher::{Watcher, watch};

use macafon_ipc::{
    DisplayIdentity, DisplayLayout, DisplayMode, DisplayMonitor, DisplayRotation, ProfileMonitor,
};
use std::mem::size_of;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, LUID, POINTL, WIN32_ERROR};
use windows::Win32::Graphics::Gdi::{DEVMODEW, ENUM_DISPLAY_SETTINGS_MODE, EnumDisplaySettingsW};
use windows::Win32::Graphics::Gdi::{
    DISPLAYCONFIG_PATH_ACTIVE, DISPLAYCONFIG_PATH_MODE_IDX_INVALID,
};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY, RegGetValueW};
use windows::core::{HRESULT, PCWSTR, w};

#[derive(Debug, thiserror::Error)]
pub enum DisplayError {
    #[error("Windows display call failed: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("Windows rejected this layout: {0}")]
    Rejected(String),
    #[error("none of this profile's monitors are connected")]
    NothingToTurnOn,
}

/// Raw active CCD configuration plus advanced-color state for exact temporary rollback.
#[derive(Clone)]
pub struct Snapshot {
    paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    modes: Vec<DISPLAYCONFIG_MODE_INFO>,
    hdr: Vec<(LUID, u32, bool)>,
}
impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Snapshot")
            .field("paths", &self.paths.len())
            .field("modes", &self.modes.len())
            .finish_non_exhaustive()
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    pub missing: Vec<String>,
}

fn check(code: i32) -> Result<(), DisplayError> {
    WIN32_ERROR(code as u32).ok().map_err(DisplayError::Windows)
}
fn rejected(code: i32) -> DisplayError {
    DisplayError::Rejected(format!(
        "{} (Win32 {code})",
        windows::core::Error::from_hresult(HRESULT::from_win32(code as u32))
    ))
}
fn text(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|v| *v == 0).unwrap_or(value.len())])
}
fn same_adapter(a: LUID, b: LUID) -> bool {
    a.LowPart == b.LowPart && a.HighPart == b.HighPart
}
fn same_source(a: &DISPLAYCONFIG_PATH_INFO, b: &DISPLAYCONFIG_PATH_INFO) -> bool {
    same_adapter(a.sourceInfo.adapterId, b.sourceInfo.adapterId)
        && a.sourceInfo.id == b.sourceInfo.id
}
fn same_target(a: &DISPLAYCONFIG_PATH_INFO, b: &DISPLAYCONFIG_PATH_INFO) -> bool {
    same_adapter(a.targetInfo.adapterId, b.targetInfo.adapterId)
        && a.targetInfo.id == b.targetInfo.id
}
fn active(p: &DISPLAYCONFIG_PATH_INFO) -> bool {
    p.flags & DISPLAYCONFIG_PATH_ACTIVE != 0
}
fn header<T>(
    kind: DISPLAYCONFIG_DEVICE_INFO_TYPE,
    adapter: LUID,
    id: u32,
) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: kind,
        size: size_of::<T>() as u32,
        adapterId: adapter,
        id,
    }
}
fn query(flags: QUERY_DISPLAY_CONFIG_FLAGS) -> Result<Snapshot, DisplayError> {
    for _ in 0..8 {
        let (mut np, mut nm) = (0, 0);
        // SAFETY: output counts are valid writable pointers.
        unsafe {
            GetDisplayConfigBufferSizes(flags, &mut np, &mut nm).ok()?;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        // SAFETY: buffers have the sizes obtained above. No database query needs a topology pointer.
        let code = unsafe {
            QueryDisplayConfig(
                flags,
                &mut np,
                paths.as_mut_ptr(),
                &mut nm,
                modes.as_mut_ptr(),
                None,
            )
        };
        if code == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        code.ok()?;
        paths.truncate(np as usize);
        modes.truncate(nm as usize);
        return Ok(Snapshot {
            paths,
            modes,
            hdr: Vec::new(),
        });
    }
    Err(DisplayError::Rejected(
        "display topology kept changing during enumeration".into(),
    ))
}
fn color(path: &DISPLAYCONFIG_PATH_INFO) -> (bool, bool) {
    let mut info = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
        header: header::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(
            DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
            path.targetInfo.adapterId,
            path.targetInfo.id,
        ),
        ..Default::default()
    };
    // SAFETY: header describes the complete writable info packet; value is its documented bitfield.
    unsafe {
        if DisplayConfigGetDeviceInfo(&mut info.header) != 0 {
            return (false, false);
        }
        let bits = info.Anonymous.value;
        (bits & 1 != 0 && bits & 8 == 0, bits & 2 != 0)
    }
}
fn set_color(adapter: LUID, id: u32, enabled: bool) -> Result<(), DisplayError> {
    let info = DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE {
        header: header::<DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE>(
            DISPLAYCONFIG_DEVICE_INFO_SET_ADVANCED_COLOR_STATE,
            adapter,
            id,
        ),
        Anonymous: DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE_0 {
            value: u32::from(enabled),
        },
    };
    // SAFETY: header size/type describe the full immutable advanced-color packet.
    check(unsafe { DisplayConfigSetDeviceInfo(&info.header) })
}
fn preferred(path: &DISPLAYCONFIG_PATH_INFO) -> Option<DISPLAYCONFIG_TARGET_PREFERRED_MODE> {
    let mut info = DISPLAYCONFIG_TARGET_PREFERRED_MODE {
        header: header::<DISPLAYCONFIG_TARGET_PREFERRED_MODE>(
            DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE,
            path.targetInfo.adapterId,
            path.targetInfo.id,
        ),
        ..Default::default()
    };
    // SAFETY: a correctly sized packet is provided for this request.
    (unsafe { DisplayConfigGetDeviceInfo(&mut info.header) } == 0).then_some(info)
}
fn mhz(rate: DISPLAYCONFIG_RATIONAL) -> u32 {
    if rate.Denominator == 0 {
        0
    } else {
        ((u64::from(rate.Numerator) * 1000 + u64::from(rate.Denominator) / 2)
            / u64::from(rate.Denominator))
        .min(u64::from(u32::MAX)) as u32
    }
}
fn signal_mode(target: DISPLAYCONFIG_TARGET_MODE) -> DisplayMode {
    let signal = target.targetVideoSignalInfo;
    DisplayMode {
        width: signal.activeSize.cx,
        height: signal.activeSize.cy,
        refresh_mhz: mhz(signal.vSyncFreq),
    }
}
fn rotation(value: DISPLAYCONFIG_ROTATION) -> DisplayRotation {
    match value {
        DISPLAYCONFIG_ROTATION_ROTATE90 => DisplayRotation::Rotate90,
        DISPLAYCONFIG_ROTATION_ROTATE180 => DisplayRotation::Rotate180,
        DISPLAYCONFIG_ROTATION_ROTATE270 => DisplayRotation::Rotate270,
        _ => DisplayRotation::None,
    }
}
fn native_rotation(value: DisplayRotation) -> DISPLAYCONFIG_ROTATION {
    match value {
        DisplayRotation::None => DISPLAYCONFIG_ROTATION_IDENTITY,
        DisplayRotation::Rotate90 => DISPLAYCONFIG_ROTATION_ROTATE90,
        DisplayRotation::Rotate180 => DISPLAYCONFIG_ROTATION_ROTATE180,
        DisplayRotation::Rotate270 => DISPLAYCONFIG_ROTATION_ROTATE270,
    }
}
fn dimensions(l: &DisplayLayout) -> (u32, u32) {
    if matches!(
        l.rotation,
        DisplayRotation::Rotate90 | DisplayRotation::Rotate270
    ) {
        (l.mode.height, l.mode.width)
    } else {
        (l.mode.width, l.mode.height)
    }
}

/// Read only the EDID belonging to this exact monitor interface's device instance.
/// Bad checksums, absent serials and placeholder serials remain None; never use a port id as a serial.
fn edid(path: &str) -> Option<Vec<u8>> {
    let parts: Vec<_> = path.strip_prefix(r"\\?\")?.split('#').collect();
    if parts.len() != 4 || !parts[0].eq_ignore_ascii_case("DISPLAY") {
        return None;
    }
    let key: Vec<_> = format!(
        r"SYSTEM\CurrentControlSet\Enum\{}\{}\{}\Device Parameters",
        parts[0], parts[1], parts[2]
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    let mut bytes = vec![0u8; 4096];
    let mut size = bytes.len() as u32;
    // SAFETY: null-terminated strings and an allocated byte buffer with its actual capacity.
    let code = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            w!("EDID"),
            RRF_RT_REG_BINARY,
            None,
            Some(bytes.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if code.0 != 0 || size < 128 {
        return None;
    }
    bytes.truncate(size as usize);
    if bytes[..8] != [0, 255, 255, 255, 255, 255, 255, 0]
        || bytes[..128].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) != 0
    {
        return None;
    }
    Some(bytes)
}
fn serial(bytes: &[u8]) -> Option<String> {
    for descriptor in bytes[54..126].chunks_exact(18) {
        if descriptor[..5] == [0, 0, 0, 255, 0] {
            let value = String::from_utf8_lossy(&descriptor[5..18])
                .trim_matches([' ', '\n', '\r', '\0'])
                .to_string();
            if !value.is_empty()
                && value.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
                && value.bytes().any(|b| b != b'0' && b != b' ')
            {
                return Some(value);
            }
        }
    }
    let value = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    (value != 0 && value != u32::MAX).then(|| value.to_string())
}
fn manufacturer(value: u16) -> String {
    [10, 5, 0]
        .into_iter()
        .map(|shift| {
            let c = ((value >> shift) & 31) as u8;
            if (1..=26).contains(&c) {
                char::from(b'A' + c - 1)
            } else {
                '?'
            }
        })
        .collect()
}
fn connector(technology: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY) -> &'static str {
    match technology {
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
        | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
        | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED
        | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS => "Built-in display",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI => "HDMI display",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EXTERNAL
        | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_USB_TUNNEL => "DisplayPort display",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DVI => "DVI display",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HD15 => "VGA display",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_MIRACAST => "Wireless display",
        _ => "Display",
    }
}
struct Inventory {
    config: Snapshot,
    monitors: Vec<DisplayMonitor>,
    paths: Vec<DISPLAYCONFIG_PATH_INFO>,
}
fn inventory() -> Result<Inventory, DisplayError> {
    let config = query(QDC_ALL_PATHS)?;
    let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = Vec::new();
    // Prefer the active source-target combination for each physical target.
    for p in config
        .paths
        .iter()
        .filter(|p| p.targetInfo.targetAvailable.as_bool())
    {
        if let Some(i) = paths.iter().position(|old| same_target(old, p)) {
            if active(p) {
                paths[i] = *p;
            }
        } else {
            paths.push(*p);
        }
    }
    let mut monitors = Vec::new();
    for p in &paths {
        let mut name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
            header: header::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(
                DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                p.targetInfo.adapterId,
                p.targetInfo.id,
            ),
            ..Default::default()
        };
        // SAFETY: correctly sized target-name request, written synchronously by CCD.
        check(unsafe { DisplayConfigGetDeviceInfo(&mut name.header) })?;
        let path = text(&name.monitorDevicePath);
        if path.is_empty() {
            return Err(DisplayError::Rejected(
                "Windows returned a monitor without a stable device path".into(),
            ));
        }
        let bytes = edid(&path);
        // Prefer checksum-verified EDID; the interface hardware id also carries the
        // canonical PNP manufacturer+product string without an endian conversion.
        let hardware = path
            .split('#')
            .nth(1)
            .filter(|s| {
                s.len() == 7
                    && s.as_bytes()[..3].iter().all(u8::is_ascii_alphabetic)
                    && s.as_bytes()[3..].iter().all(u8::is_ascii_hexdigit)
            })
            .map(str::to_ascii_uppercase);
        let edid_id = bytes
            .as_ref()
            .map(|b| {
                format!(
                    "{}{:04X}",
                    manufacturer(u16::from_be_bytes([b[8], b[9]])),
                    u16::from_le_bytes([b[10], b[11]])
                )
            })
            .or(hardware)
            .unwrap_or_else(|| {
                // SAFETY: the name packet was filled by CCD; this is its flags bitfield.
                if unsafe { name.flags.Anonymous.value } & 4 != 0 {
                    format!(
                        "{}{:04X}",
                        manufacturer(name.edidManufactureId.swap_bytes()),
                        name.edidProductCodeId
                    )
                } else {
                    String::new() // Unknown identities must only match by their exact path.
                }
            });
        let (hdr_supported, hdr) = color(p);
        let mut modes = Vec::new();
        if let Some(preferred) = preferred(p) {
            modes.push(signal_mode(preferred.targetMode));
        }
        let layout = if active(p) {
            // SAFETY: queries omit virtual-mode flags, so these unions contain modeInfoIdx.
            let (si, ti) = unsafe {
                (
                    p.sourceInfo.Anonymous.modeInfoIdx,
                    p.targetInfo.Anonymous.modeInfoIdx,
                )
            };
            let source = config
                .modes
                .get(si as usize)
                .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
                .ok_or_else(|| {
                    DisplayError::Rejected("active source mode is unavailable".into())
                })?;
            let target = config
                .modes
                .get(ti as usize)
                .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_TARGET)
                .ok_or_else(|| {
                    DisplayError::Rejected("active target mode is unavailable".into())
                })?;
            // SAFETY: union discriminants were checked above.
            let (source, target) =
                unsafe { (source.Anonymous.sourceMode, target.Anonymous.targetMode) };
            let mut mode = signal_mode(target);
            let refresh = mhz(p.targetInfo.refreshRate);
            if refresh != 0 {
                mode.refresh_mhz = refresh;
            }
            modes.push(mode);
            let mut source_name = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: header::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(
                    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    p.sourceInfo.adapterId,
                    p.sourceInfo.id,
                ),
                ..Default::default()
            };
            // SAFETY: correctly sized source-name packet.
            if unsafe { DisplayConfigGetDeviceInfo(&mut source_name.header) } == 0 {
                for index in 0..4096 {
                    let mut dm = DEVMODEW {
                        dmSize: size_of::<DEVMODEW>() as u16,
                        ..Default::default()
                    };
                    // SAFETY: source GDI name is null terminated, dm has its required size.
                    if !unsafe {
                        EnumDisplaySettingsW(
                            PCWSTR(source_name.viewGdiDeviceName.as_ptr()),
                            ENUM_DISPLAY_SETTINGS_MODE(index),
                            &mut dm,
                        )
                    }
                    .as_bool()
                    {
                        break;
                    }
                    let (mut width, mut height) = (dm.dmPelsWidth, dm.dmPelsHeight);
                    // SAFETY: EnumDisplaySettings initialized the display arm of DEVMODEW.
                    let orientation = unsafe { dm.Anonymous1.Anonymous2.dmDisplayOrientation.0 };
                    if orientation == 1 || orientation == 3 {
                        std::mem::swap(&mut width, &mut height);
                    }
                    if width > 0 && height > 0 && dm.dmDisplayFrequency > 1 {
                        modes.push(DisplayMode {
                            width,
                            height,
                            refresh_mhz: dm.dmDisplayFrequency.saturating_mul(1000),
                        });
                    }
                }
            }
            Some(DisplayLayout {
                x: source.position.x,
                y: source.position.y,
                mode,
                rotation: rotation(p.targetInfo.rotation),
                primary: source.position.x == 0 && source.position.y == 0,
                mirror_of: None,
                hdr,
            })
        } else {
            None
        };
        modes.retain(|m| m.width > 0 && m.height > 0 && m.refresh_mhz > 0);
        modes.sort_by_key(|m| {
            std::cmp::Reverse((
                u64::from(m.width) * u64::from(m.height),
                m.width,
                m.height,
                m.refresh_mhz,
            ))
        });
        modes.dedup();
        let friendly = text(&name.monitorFriendlyDeviceName);
        monitors.push(DisplayMonitor {
            identity: DisplayIdentity {
                edid: edid_id,
                serial: bytes.as_ref().and_then(|b| serial(b)),
                path,
            },
            name: if friendly.is_empty() {
                connector(p.targetInfo.outputTechnology).into()
            } else {
                friendly
            },
            built_in: matches!(
                p.targetInfo.outputTechnology,
                DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
                    | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
                    | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED
                    | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS
            ),
            layout,
            modes,
            hdr_supported,
        });
    }
    for i in 0..monitors.len() {
        if active(&paths[i]) {
            // Prefer a target whose oriented resolution equals the desktop source.
            // In a scaled clone the first enumerated target can have a smaller mode.
            // SAFETY: this legacy query stores a source mode index in the union.
            let source_index = unsafe { paths[i].sourceInfo.Anonymous.modeInfoIdx };
            let source_dimensions = config
                .modes
                .get(source_index as usize)
                .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
                .map(|m| {
                    // SAFETY: infoType was checked immediately above.
                    let source = unsafe { m.Anonymous.sourceMode };
                    (source.width, source.height)
                });
            let members: Vec<_> = (0..monitors.len())
                .filter(|j| active(&paths[*j]) && same_source(&paths[i], &paths[*j]))
                .collect();
            let root = members
                .iter()
                .copied()
                .find(|j| monitors[*j].layout.as_ref().map(dimensions) == source_dimensions)
                .or_else(|| members.first().copied());
            if let Some(j) = root.filter(|j| *j != i) {
                let path = monitors[j].identity.path.clone();
                if let Some(l) = &mut monitors[i].layout {
                    l.mirror_of = Some(path);
                    l.primary = false;
                }
            }
        }
    }
    Ok(Inventory {
        config,
        monitors,
        paths,
    })
}

pub fn monitors() -> Result<Vec<DisplayMonitor>, DisplayError> {
    Ok(inventory()?.monitors)
}
pub fn capture() -> Result<Vec<ProfileMonitor>, DisplayError> {
    Ok(monitors()?
        .into_iter()
        .map(|m| ProfileMonitor {
            identity: m.identity,
            name: m.name,
            layout: m.layout,
        })
        .collect())
}
pub fn snapshot() -> Result<Snapshot, DisplayError> {
    let mut snapshot = query(QDC_ONLY_ACTIVE_PATHS)?;
    for p in &snapshot.paths {
        let (supported, enabled) = color(p);
        if supported {
            snapshot
                .hdr
                .push((p.targetInfo.adapterId, p.targetInfo.id, enabled));
        }
    }
    Ok(snapshot)
}
pub fn restore(snapshot: &Snapshot) -> Result<(), DisplayError> {
    // SAFETY: immutable raw CCD arrays come from a successful active-configuration query.
    check(unsafe {
        SetDisplayConfig(
            Some(&snapshot.paths),
            Some(&snapshot.modes),
            SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_SAVE_TO_DATABASE,
        )
    })?;
    for (adapter, id, enabled) in &snapshot.hdr {
        let path = DISPLAYCONFIG_PATH_INFO {
            targetInfo: DISPLAYCONFIG_PATH_TARGET_INFO {
                adapterId: *adapter,
                id: *id,
                ..Default::default()
            },
            ..Default::default()
        };
        let (supported, current) = color(&path);
        if supported && current != *enabled {
            set_color(*adapter, *id, *enabled)?;
        }
    }
    Ok(())
}

struct Plan {
    config: Snapshot,
    missing: Vec<String>,
    hdr: Vec<(LUID, u32, bool)>,
}

/// Assign one unique source per extended group, a shared source within a mirror group.
/// Backtracking matters on adapters whose targets support different source subsets.
fn assign(
    groups: &[Vec<usize>],
    candidates: &[Vec<DISPLAYCONFIG_PATH_INFO>],
    group: usize,
    chosen: &mut Vec<Vec<DISPLAYCONFIG_PATH_INFO>>,
) -> bool {
    if group == groups.len() {
        return true;
    }
    let members = &groups[group];
    for source in &candidates[members[0]] {
        if chosen.iter().any(|g| same_source(&g[0], source)) {
            continue;
        }
        let paths: Option<Vec<_>> = members
            .iter()
            .map(|i| {
                candidates[*i]
                    .iter()
                    .find(|p| same_source(p, source))
                    .copied()
            })
            .collect();
        if let Some(paths) = paths {
            chosen.push(paths);
            if assign(groups, candidates, group + 1, chosen) {
                return true;
            }
            chosen.pop();
        }
    }
    false
}
fn plan(profile: &[ProfileMonitor]) -> Result<Plan, DisplayError> {
    let live = inventory()?;
    build_plan(profile, live)
}
fn build_plan(profile: &[ProfileMonitor], live: Inventory) -> Result<Plan, DisplayError> {
    let resolved = logic::resolve(profile, &live.monitors);
    let missing = profile
        .iter()
        .zip(&resolved)
        .filter(|(_, j)| j.is_none())
        .map(|(p, _)| p.name.clone())
        .collect();
    if !profile
        .iter()
        .enumerate()
        .any(|(i, p)| resolved[i].is_some() && p.layout.is_some())
    {
        return Err(DisplayError::NothingToTurnOn);
    }
    let logic::Connected { layouts, groups } =
        logic::connected(profile, &resolved).map_err(|s| DisplayError::Rejected(s.into()))?;
    let candidates: Vec<Vec<_>> = resolved
        .iter()
        .map(|index| {
            index.map_or_else(Vec::new, |j| {
                let mut paths: Vec<_> = live
                    .config
                    .paths
                    .iter()
                    .filter(|p| {
                        p.targetInfo.targetAvailable.as_bool() && same_target(p, &live.paths[j])
                    })
                    .copied()
                    .collect();
                paths.sort_by_key(|p| !active(p));
                paths
            })
        })
        .collect();
    let mut selected = Vec::new();
    if !assign(&groups, &candidates, 0, &mut selected) {
        return Err(DisplayError::Rejected(
            "no compatible display sources for this extend/mirror topology".into(),
        ));
    }
    let mut config = Snapshot {
        paths: Vec::new(),
        modes: Vec::new(),
        hdr: Vec::new(),
    };
    let mut hdr = Vec::new();
    for (group, paths) in groups.iter().zip(selected) {
        let root = layouts[group[0]]
            .as_ref()
            .ok_or(DisplayError::NothingToTurnOn)?;
        let (width, height) = dimensions(root);
        if width == 0
            || height == 0
            || root.mode.refresh_mhz == 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
        {
            return Err(DisplayError::Rejected(
                "invalid display dimensions or refresh rate".into(),
            ));
        }
        let source_index = config.modes.len() as u32;
        let source = DISPLAYCONFIG_SOURCE_MODE {
            width,
            height,
            pixelFormat: DISPLAYCONFIG_PIXELFORMAT_32BPP,
            position: POINTL {
                x: root.x,
                y: root.y,
            },
        };
        config.modes.push(DISPLAYCONFIG_MODE_INFO {
            infoType: DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE,
            id: paths[0].sourceInfo.id,
            adapterId: paths[0].sourceInfo.adapterId,
            Anonymous: DISPLAYCONFIG_MODE_INFO_0 { sourceMode: source },
        });
        for (i, mut path) in group.iter().zip(paths) {
            let l = layouts[*i].as_ref().ok_or(DisplayError::NothingToTurnOn)?;
            if l.mode.width == 0 || l.mode.height == 0 || l.mode.refresh_mhz == 0 {
                return Err(DisplayError::Rejected(
                    "invalid display dimensions or refresh rate".into(),
                ));
            }
            if l.x != root.x || l.y != root.y {
                return Err(DisplayError::Rejected(
                    "mirrored monitors must have the same desktop position".into(),
                ));
            }
            path.flags = DISPLAYCONFIG_PATH_ACTIVE;
            path.sourceInfo.Anonymous.modeInfoIdx = source_index;
            path.sourceInfo.statusFlags = 0;
            path.targetInfo.Anonymous.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
            path.targetInfo.statusFlags = 0;
            path.targetInfo.rotation = native_rotation(l.rotation);
            path.targetInfo.scaling = if dimensions(l) == (width, height) {
                DISPLAYCONFIG_SCALING_IDENTITY
            } else {
                DISPLAYCONFIG_SCALING_PREFERRED
            };
            path.targetInfo.refreshRate = DISPLAYCONFIG_RATIONAL {
                Numerator: l.mode.refresh_mhz,
                Denominator: 1000,
            };
            path.targetInfo.scanLineOrdering = DISPLAYCONFIG_SCANLINE_ORDERING_UNSPECIFIED;
            // Preserve exact timings when available; otherwise CCD best-mode logic chooses
            // target timings from the supplied source dimensions and path refresh rate.
            let current = live
                .config
                .paths
                .iter()
                .find(|p| active(p) && same_target(p, &path))
                .and_then(|p| {
                    // SAFETY: the query used legacy (non-virtual) mode indices.
                    let index = unsafe { p.targetInfo.Anonymous.modeInfoIdx };
                    live.config
                        .modes
                        .get(index as usize)
                        .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_TARGET)
                        .map(|m| {
                            // SAFETY: infoType above identifies the targetMode arm.
                            unsafe { m.Anonymous.targetMode }
                        })
                });
            let target = current
                .filter(|t| logic::mode_matches(signal_mode(*t), l.mode))
                .or_else(|| {
                    preferred(&path)
                        .map(|p| p.targetMode)
                        .filter(|t| logic::mode_matches(signal_mode(*t), l.mode))
                });
            if target.is_none() && dimensions(l) != (width, height) {
                return Err(DisplayError::Rejected(
                    "scaled mirror timing is unavailable: use its current or preferred target mode"
                        .into(),
                ));
            }
            if let Some(target) = target {
                path.targetInfo.Anonymous.modeInfoIdx = config.modes.len() as u32;
                config.modes.push(DISPLAYCONFIG_MODE_INFO {
                    infoType: DISPLAYCONFIG_MODE_INFO_TYPE_TARGET,
                    id: path.targetInfo.id,
                    adapterId: path.targetInfo.adapterId,
                    Anonymous: DISPLAYCONFIG_MODE_INFO_0 { targetMode: target },
                });
            }
            hdr.push((path.targetInfo.adapterId, path.targetInfo.id, l.hdr));
            config.paths.push(path);
        }
    }
    Ok(Plan {
        config,
        missing,
        hdr,
    })
}
fn validate_plan(plan: &Plan) -> Result<(), DisplayError> {
    // SAFETY: all indices reference the supplied arrays or the documented invalid sentinel.
    let code = unsafe {
        SetDisplayConfig(
            Some(&plan.config.paths),
            Some(&plan.config.modes),
            SDC_VALIDATE | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES,
        )
    };
    if code == 0 {
        Ok(())
    } else {
        Err(rejected(code))
    }
}
/// Resolve and validate without modifying the display configuration or HDR state.
pub fn validate(profile: &[ProfileMonitor]) -> Result<Applied, DisplayError> {
    let plan = plan(profile)?;
    validate_plan(&plan)?;
    Ok(Applied {
        missing: plan.missing,
    })
}
pub fn apply(profile: &[ProfileMonitor]) -> Result<Applied, DisplayError> {
    let plan = plan(profile)?;
    validate_plan(&plan)?;
    // SAFETY: this exact topology was validated; Windows may refine driver timings.
    check(unsafe {
        SetDisplayConfig(
            Some(&plan.config.paths),
            Some(&plan.config.modes),
            SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_SAVE_TO_DATABASE | SDC_ALLOW_CHANGES,
        )
    })?;
    for (adapter, id, enabled) in plan.hdr {
        let path = DISPLAYCONFIG_PATH_INFO {
            targetInfo: DISPLAYCONFIG_PATH_TARGET_INFO {
                adapterId: adapter,
                id,
                ..Default::default()
            },
            ..Default::default()
        };
        let (supported, current) = color(&path);
        if supported && current != enabled {
            set_color(adapter, id, enabled)?;
        }
    }
    tracing::info!(missing = plan.missing.len(), "Display profile applied");
    Ok(Applied {
        missing: plan.missing,
    })
}
pub fn all_on() -> Result<(), DisplayError> {
    // SAFETY: topology flags require null arrays; no caller-provided pointers are used.
    check(unsafe { SetDisplayConfig(None, None, SDC_TOPOLOGY_EXTEND | SDC_APPLY) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(source: u32, target: u32) -> DISPLAYCONFIG_PATH_INFO {
        DISPLAYCONFIG_PATH_INFO {
            sourceInfo: DISPLAYCONFIG_PATH_SOURCE_INFO {
                id: source,
                ..Default::default()
            },
            targetInfo: DISPLAYCONFIG_PATH_TARGET_INFO {
                id: target,
                targetAvailable: true.into(),
                ..Default::default()
            },
            flags: DISPLAYCONFIG_PATH_ACTIVE,
        }
    }

    #[test]
    fn displays_source_assignment_backtracks_and_shares_mirrors() {
        let candidates = vec![vec![path(0, 0), path(1, 0)], vec![path(0, 1)]];
        let mut chosen = Vec::new();
        assert!(assign(&[vec![0], vec![1]], &candidates, 0, &mut chosen));
        assert_eq!(chosen[0][0].sourceInfo.id, 1);
        assert_eq!(chosen[1][0].sourceInfo.id, 0);
        chosen.clear();
        assert!(assign(&[vec![0, 1]], &candidates, 0, &mut chosen));
        assert!(same_source(&chosen[0][0], &chosen[0][1]));
    }

    #[test]
    fn displays_plan_preserves_scaled_mirror_timings_and_normalizes_primary() {
        let mut config = Snapshot {
            paths: Vec::new(),
            modes: Vec::new(),
            hdr: Vec::new(),
        };
        let mut monitors = Vec::new();
        // Current cloned desktop: same source, different physical target resolutions.
        for (i, width) in [1920, 1280].into_iter().enumerate() {
            let mut path = path(0, i as u32);
            path.sourceInfo.Anonymous.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
            path.targetInfo.Anonymous.modeInfoIdx = i as u32;
            let target = DISPLAYCONFIG_TARGET_MODE {
                targetVideoSignalInfo: DISPLAYCONFIG_VIDEO_SIGNAL_INFO {
                    activeSize: DISPLAYCONFIG_2DREGION {
                        cx: width,
                        cy: 1080,
                    },
                    vSyncFreq: DISPLAYCONFIG_RATIONAL {
                        Numerator: 60,
                        Denominator: 1,
                    },
                    ..Default::default()
                },
            };
            config.paths.push(path);
            config.modes.push(DISPLAYCONFIG_MODE_INFO {
                infoType: DISPLAYCONFIG_MODE_INFO_TYPE_TARGET,
                id: i as u32,
                Anonymous: DISPLAYCONFIG_MODE_INFO_0 { targetMode: target },
                ..Default::default()
            });
            monitors.push(DisplayMonitor {
                identity: DisplayIdentity {
                    edid: format!("TEST{i}"),
                    serial: None,
                    path: format!("monitor{i}"),
                },
                name: format!("Monitor {i}"),
                built_in: false,
                hdr_supported: false,
                modes: Vec::new(),
                layout: Some(DisplayLayout {
                    x: 100,
                    y: 50,
                    mode: signal_mode(target),
                    rotation: DisplayRotation::None,
                    primary: i == 0,
                    mirror_of: (i == 1).then(|| "monitor0".into()),
                    hdr: false,
                }),
            });
        }
        let profile: Vec<_> = monitors
            .iter()
            .map(|m| ProfileMonitor {
                identity: m.identity.clone(),
                name: m.name.clone(),
                layout: m.layout.clone(),
            })
            .collect();
        let inventory = Inventory {
            paths: config.paths.clone(),
            config,
            monitors,
        };
        let planned = build_plan(&profile, inventory).expect("scaled mirror plan");
        assert_eq!(planned.config.paths.len(), 2);
        assert_eq!(planned.config.modes.len(), 3); // one source, two distinct target timings
        assert!(same_source(
            &planned.config.paths[0],
            &planned.config.paths[1]
        ));
        assert_eq!(
            planned.config.paths[1].targetInfo.scaling,
            DISPLAYCONFIG_SCALING_PREFERRED
        );
        // SAFETY: the planner emits a source mode first and legacy mode indices on both paths.
        unsafe {
            assert_eq!(
                planned.config.modes[0].Anonymous.sourceMode.position,
                POINTL { x: 0, y: 0 }
            );
            assert_eq!(
                planned.config.paths[0].sourceInfo.Anonymous.modeInfoIdx,
                planned.config.paths[1].sourceInfo.Anonymous.modeInfoIdx
            );
            assert_eq!(
                planned.config.modes[2]
                    .Anonymous
                    .targetMode
                    .targetVideoSignalInfo
                    .activeSize
                    .cx,
                1280
            );
        }
    }
}
