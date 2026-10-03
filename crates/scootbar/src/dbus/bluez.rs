//! BlueZ wire shapes: what the bluetooth module reads of `org.bluez`
//! objects, over [`super::proto`]'s readers. Like `mpris`, it uses nothing
//! but `std` and is compiled by the fuzz crate (`crates/scootbar/fuzz`)
//! unchanged.
//!
//! The same typed walk reads every shape BlueZ sends its state in: the
//! `a{oa{sa{sv}}}` a `GetManagedObjects` answers with, the `oa{sa{sv}}`
//! and `oas` bodies of `InterfacesAdded` and `InterfacesRemoved`, the
//! `a{sv}` a per-interface `GetAll` answers with, and the `sa{sv}as` body
//! of a `PropertiesChanged` signal. Every property the bar does not show
//! (a device's RSSI, its TX power, anything a later BlueZ adds) is skipped
//! by its signature, never parsed into a value.
//!
//! Strings come back borrowed and unbounded (a device can name itself a
//! kilobyte, inside the 1 MiB message the client takes); the consumer
//! cuts them where it stores them. Counts are bounded here: interfaces of
//! one object and properties of one interface past their limits refuse the
//! whole answer, so one object with a million one-byte keys is never
//! walked; objects themselves are not counted (a machine with many known
//! devices is many objects, and the message cap already bounds them), but
//! the consumer holds only the first of each kind. A property of the wrong
//! type for its name (a `Powered` that is a string) is skipped like an
//! unknown one: the object loses that property, not the whole answer. A
//! misshapen body is refused whole (`Err(())`) and the caller keeps its
//! last state.

use super::proto::{self, Reader, check_path};

#[cfg(test)]
pub(crate) mod build;
#[cfg(test)]
mod tests;

/// BlueZ's well-known name, and the root its objects live under.
pub const NAME: &str = "org.bluez";
pub const ROOT: &str = "/org/bluez";
/// The interfaces that carry the state: the manager, the properties, and
/// the three the bar reads.
pub const MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
pub const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
pub const ADAPTER: &str = "org.bluez.Adapter1";
pub const DEVICE: &str = "org.bluez.Device1";
pub const BATTERY: &str = "org.bluez.Battery1";

/// The most interfaces walked of one object: BlueZ serves about six
/// (Adapter1 or Device1, Properties, Introspectable, ObjectManager on the
/// root). Past this the whole answer is refused.
pub const MAX_INTERFACES: usize = 32;
/// The most names read from a signal's added, removed or invalidated
/// list.
pub const MAX_NAMES: usize = 128;

/// What the bar takes of an `Adapter1` interface: whether it is powered.
/// `None` when BlueZ did not send it (or sent it with a type it does not
/// have).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AdapterProps {
    pub powered: Option<bool>,
}

/// What the bar takes of a `Device1` interface: whether it is connected,
/// and the name shown for it (`Name`, else `Alias`). Each is `None` when
/// BlueZ did not send it (or sent it with a type it does not have).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DeviceProps<'a> {
    pub connected: Option<bool>,
    pub name: Option<&'a str>,
    pub alias: Option<&'a str>,
}

/// What the bar takes of a `Battery1` interface: the charge in percent.
/// `None` when BlueZ did not send it (or sent it with a type it does not
/// have).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BatteryProps {
    pub percentage: Option<u8>,
}

/// One object of a managed set: its path and the interfaces the bar
/// reads, each `None` when the object does not serve it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Object<'a> {
    pub path: &'a str,
    pub adapter: Option<AdapterProps>,
    pub device: Option<DeviceProps<'a>>,
    pub battery: Option<BatteryProps>,
}

/// Which interface a `PropertiesChanged` signal is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Iface {
    Adapter,
    Device,
    Battery,
    /// Any other interface: the signal is not the bar's to walk.
    #[default]
    Other,
}

/// A `PropertiesChanged` signal's body: the interface's properties, and
/// whether one the bar holds was invalidated (the signal said it changed
/// without saying to what, so the bar must ask).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changed<'a> {
    pub iface: Iface,
    pub adapter: AdapterProps,
    pub device: DeviceProps<'a>,
    pub battery: BatteryProps,
    pub invalidated: bool,
}

impl<'a> Changed<'a> {
    fn other() -> Self {
        Self::default()
    }
}

/// Reads a `GetManagedObjects` body (`a{oa{sa{sv}}}`). `Err(())` refuses
/// the whole answer.
pub fn read_managed_objects(body: &[u8]) -> Result<Vec<Object<'_>>, ()> {
    let mut reader = Reader::le(body);
    let mut objects = reader.elements(8)?;
    let mut out = Vec::new();
    while !objects.exhausted() {
        objects.enter_struct()?;
        let path = objects.str()?;
        check_path(path)?;
        let mut ifaces = objects.elements(8)?;
        let object = read_object_ifaces(path, &mut ifaces)?;
        objects.leave_struct();
        out.push(object);
    }
    if !reader.exhausted() {
        return Err(());
    }
    Ok(out)
}

/// Reads an `InterfacesAdded` body (`oa{sa{sv}}`): the object that gained
/// interfaces. `Err(())` refuses the whole signal.
pub fn read_interfaces_added(body: &[u8]) -> Result<Object<'_>, ()> {
    let mut reader = Reader::le(body);
    let path = reader.str()?;
    check_path(path)?;
    let mut ifaces = reader.elements(8)?;
    let object = read_object_ifaces(path, &mut ifaces)?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok(object)
}

/// Reads an `InterfacesRemoved` body (`oas`): the object path and the
/// interface names it lost. `Err(())` refuses the whole signal.
pub fn read_interfaces_removed(body: &[u8]) -> Result<(&str, Vec<&str>), ()> {
    let mut reader = Reader::le(body);
    let path = reader.str()?;
    check_path(path)?;
    let mut names = reader.elements(4)?;
    let mut out = Vec::new();
    while !names.exhausted() {
        if out.len() >= MAX_NAMES {
            return Err(());
        }
        let name = names.str()?;
        check_interface_lenient(name)?;
        out.push(name);
    }
    if !reader.exhausted() {
        return Err(());
    }
    Ok((path, out))
}

/// An interface name is read but never matched on except against the
/// three the bar knows: overlong or ill-formed ones are kept (a removal
/// is matched by equality) but must still fit the bus.
fn check_interface_lenient(name: &str) -> Result<(), ()> {
    if name.is_empty() || name.len() > 255 {
        return Err(());
    }
    Ok(())
}

/// Reads one object's interface dictionary (`a{sa{sv}}`) into an
/// [`Object`]: the three interfaces the bar reads typed, every other one
/// walked past entry by entry.
fn read_object_ifaces<'a>(path: &'a str, ifaces: &mut Reader<'a>) -> Result<Object<'a>, ()> {
    let mut object = Object {
        path,
        ..Object::default()
    };
    let mut seen = 0;
    while !ifaces.exhausted() {
        seen += 1;
        if seen > MAX_INTERFACES {
            return Err(());
        }
        ifaces.enter_struct()?;
        let name = ifaces.str()?;
        match name {
            ADAPTER => {
                let mut props = ifaces.elements(8)?;
                object.adapter = Some(walk_adapter(&mut props)?);
            }
            DEVICE => {
                let mut props = ifaces.elements(8)?;
                object.device = Some(walk_device(&mut props)?);
            }
            BATTERY => {
                let mut props = ifaces.elements(8)?;
                object.battery = Some(walk_battery(&mut props)?);
            }
            _ => {
                let mut props = ifaces.elements(8)?;
                skip_dict(&mut props)?;
            }
        }
        ifaces.leave_struct();
    }
    Ok(object)
}

/// Reads a `GetAll` body (`a{sv}`) for `Adapter1`. `Err(())` refuses the
/// whole answer.
pub fn read_adapter_all(body: &[u8]) -> Result<AdapterProps, ()> {
    let mut reader = Reader::le(body);
    let mut entries = reader.elements(8)?;
    let props = walk_adapter(&mut entries)?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok(props)
}

/// Reads a `GetAll` body (`a{sv}`) for `Device1`. `Err(())` refuses the
/// whole answer.
pub fn read_device_all(body: &[u8]) -> Result<DeviceProps<'_>, ()> {
    let mut reader = Reader::le(body);
    let mut entries = reader.elements(8)?;
    let props = walk_device(&mut entries)?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok(props)
}

/// Reads a `GetAll` body (`a{sv}`) for `Battery1`. `Err(())` refuses the
/// whole answer.
pub fn read_battery_all(body: &[u8]) -> Result<BatteryProps, ()> {
    let mut reader = Reader::le(body);
    let mut entries = reader.elements(8)?;
    let props = walk_battery(&mut entries)?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok(props)
}

/// One property dictionary's entries, to its end: the `Adapter1`
/// properties the bar shows typed, every other one (and one of the wrong
/// type) skipped by its signature.
fn walk_adapter(entries: &mut Reader<'_>) -> Result<AdapterProps, ()> {
    let mut props = AdapterProps::default();
    let mut seen = 0;
    while !entries.exhausted() {
        seen += 1;
        if seen > proto::MAX_PROPERTIES {
            return Err(());
        }
        entries.enter_struct()?;
        let key = entries.str()?;
        let sig = entries.signature()?;
        match (key, sig) {
            ("Powered", "b") => props.powered = Some(entries.boolean()?),
            _ => entries.skip(sig)?,
        }
        entries.leave_struct();
    }
    Ok(props)
}

/// One property dictionary's entries, to its end: the `Device1`
/// properties the bar shows typed, the rest skipped.
fn walk_device<'a>(entries: &mut Reader<'a>) -> Result<DeviceProps<'a>, ()> {
    let mut props = DeviceProps::default();
    let mut seen = 0;
    while !entries.exhausted() {
        seen += 1;
        if seen > proto::MAX_PROPERTIES {
            return Err(());
        }
        entries.enter_struct()?;
        let key = entries.str()?;
        let sig = entries.signature()?;
        match (key, sig) {
            ("Connected", "b") => props.connected = Some(entries.boolean()?),
            ("Name", "s") => props.name = Some(entries.str()?),
            ("Alias", "s") => props.alias = Some(entries.str()?),
            _ => entries.skip(sig)?,
        }
        entries.leave_struct();
    }
    Ok(props)
}

/// One property dictionary's entries, to its end: the `Battery1`
/// properties the bar shows typed, the rest skipped.
fn walk_battery(entries: &mut Reader<'_>) -> Result<BatteryProps, ()> {
    let mut props = BatteryProps::default();
    let mut seen = 0;
    while !entries.exhausted() {
        seen += 1;
        if seen > proto::MAX_PROPERTIES {
            return Err(());
        }
        entries.enter_struct()?;
        let key = entries.str()?;
        let sig = entries.signature()?;
        match (key, sig) {
            ("Percentage", "y") => props.percentage = Some(entries.u8()?),
            _ => entries.skip(sig)?,
        }
        entries.leave_struct();
    }
    Ok(props)
}

/// One property dictionary's entries, to its end, walked past: what
/// unknown interfaces cost, entry by entry, so a malformed one still
/// refuses the answer.
fn skip_dict(entries: &mut Reader<'_>) -> Result<(), ()> {
    let mut seen = 0;
    while !entries.exhausted() {
        seen += 1;
        if seen > proto::MAX_PROPERTIES {
            return Err(());
        }
        entries.enter_struct()?;
        let _ = entries.str()?;
        let sig = entries.signature()?;
        entries.skip(sig)?;
        entries.leave_struct();
    }
    Ok(())
}

/// Reads a `PropertiesChanged` body (`sa{sv}as`). A signal for another
/// interface is answered after its first string: its dictionary is not
/// the bar's to walk.
pub fn read_properties_changed(body: &[u8]) -> Result<Changed<'_>, ()> {
    let mut reader = Reader::le(body);
    let interface = reader.str()?;
    let mut changed = match interface {
        ADAPTER => {
            let mut entries = reader.elements(8)?;
            Changed {
                iface: Iface::Adapter,
                adapter: walk_adapter(&mut entries)?,
                ..Changed::default()
            }
        }
        DEVICE => {
            let mut entries = reader.elements(8)?;
            Changed {
                iface: Iface::Device,
                device: walk_device(&mut entries)?,
                ..Changed::default()
            }
        }
        BATTERY => {
            let mut entries = reader.elements(8)?;
            Changed {
                iface: Iface::Battery,
                battery: walk_battery(&mut entries)?,
                ..Changed::default()
            }
        }
        _ => return Ok(Changed::other()),
    };
    let mut names = reader.elements(4)?;
    let mut seen = 0;
    while !names.exhausted() {
        seen += 1;
        if seen > MAX_NAMES {
            return Err(());
        }
        changed.invalidated |= match changed.iface {
            Iface::Adapter => names.str()? == "Powered",
            Iface::Device => matches!(names.str()?, "Connected" | "Name" | "Alias"),
            Iface::Battery => names.str()? == "Percentage",
            Iface::Other => {
                let _ = names.str()?;
                false
            }
        };
    }
    if !reader.exhausted() {
        return Err(());
    }
    Ok(changed)
}
