use std::path::PathBuf;

use udev::Enumerator;

pub fn find_hidraw_device(d1: u32, d2: u32) -> Result<Option<PathBuf>, std::io::Error> {
    let mut e = Enumerator::new()?;
    e.match_subsystem("hidraw")?;

    for dev in e.scan_devices()? {
        let hid_id = dev.parent_with_subsystem("hid")?.and_then(|p| p.property_value("HID_ID").and_then(|v| Some(v.to_os_string())));
        if let Some(id) = hid_id {
            let res: Vec<_> = id.to_str().unwrap().split(':').map(|p| u32::from_str_radix(p, 16).unwrap_or(0)).collect();
            match res.as_slice() {
                [_, v1, v2] if *v1 == d1 && *v2 == d2 => {
                    return Ok(dev.devnode().map_or(None, |v| Some(v.to_path_buf())));
                }
                _ => {}
            }
        }
    }
    return Ok(None);
}
