use std::io;
use std::path::PathBuf;

use lan_kvm_core::secure::Key;

#[cfg(windows)]
fn path() -> io::Result<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|dir| PathBuf::from(dir).join("lan-kvm").join("token.bin"))
        .ok_or_else(|| io::Error::other("APPDATA não definido"))
}

#[cfg(target_os = "linux")]
fn path() -> io::Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or_else(|| io::Error::other("HOME não definido"))?;
    Ok(base.join("lan-kvm").join("token"))
}

pub fn load() -> io::Result<Option<Key>> {
    load_from(&path()?)
}

fn load_from(path: &std::path::Path) -> io::Result<Option<Key>> {
    match std::fs::read(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
        Ok(data) => decode(&data).map(Some).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "segredo do pareamento ilegível")
        }),
    }
}

pub fn save(key: &Key) -> io::Result<()> {
    let path = path()?;
    if let Some(dir) = path.parent() {
        create_private_dir(dir)?;
    }
    write_private(&path, &encode(key)?)
}

pub fn forget() -> io::Result<()> {
    match std::fs::remove_file(path()?) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(target_os = "linux")]
fn create_private_dir(dir: &std::path::Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(target_os = "linux")]
fn write_private(path: &std::path::Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(data)
}

#[cfg(target_os = "linux")]
fn encode(key: &Key) -> io::Result<Vec<u8>> {
    Ok(format!("{}\n", lan_kvm_core::pairing::to_hex(key)).into_bytes())
}

#[cfg(target_os = "linux")]
fn decode(data: &[u8]) -> Option<Key> {
    lan_kvm_core::pairing::from_hex(std::str::from_utf8(data).ok()?)
}

#[cfg(windows)]
fn create_private_dir(dir: &std::path::Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(windows)]
fn write_private(path: &std::path::Path, data: &[u8]) -> io::Result<()> {
    std::fs::write(path, data)
}

#[cfg(windows)]
fn dpapi(data: &[u8], protect: bool) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let out = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(out)
}

#[cfg(windows)]
fn encode(key: &Key) -> io::Result<Vec<u8>> {
    dpapi(key, true)
}

#[cfg(windows)]
fn decode(data: &[u8]) -> Option<Key> {
    dpapi(data, false).ok()?.try_into().ok()
}

#[cfg(test)]
mod load_tests {
    use super::*;

    #[test]
    fn missing_is_unpaired_but_unreadable_is_an_error() {
        let dir = std::env::temp_dir().join(format!("lan-kvm-keystore-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("token");
        assert_eq!(load_from(&file).unwrap(), None);
        std::fs::write(&file, b"lixo").unwrap();
        assert_eq!(
            load_from(&file).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let key: Key = std::array::from_fn(|i| 200 - i as u8);
        std::fs::write(&file, encode(&key).unwrap()).unwrap();
        assert_eq!(load_from(&file).unwrap(), Some(key));
        assert!(load_from(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn dpapi_roundtrip_and_ciphertext_differs() {
        let key: Key = std::array::from_fn(|i| i as u8 + 1);
        let sealed = encode(&key).unwrap();
        assert!(!sealed.windows(key.len()).any(|w| w == key));
        assert_eq!(decode(&sealed), Some(key));
        let mut broken = sealed.clone();
        let mid = broken.len() / 2;
        broken[mid] ^= 0xFF;
        assert_eq!(decode(&broken), None);
    }
}
