//! Test builders for BlueZ bodies: what BlueZ marshals, written with
//! this crate's [`Writer`].

use super::{ADAPTER, BATTERY, DEVICE};
use crate::dbus::proto::Writer;

/// One `a{sv}` entry: the key, the variant's signature, then the value
/// `write` makes.
pub fn entry(body: &mut Writer, key: &str, sig: &str, write: &dyn Fn(&mut Writer)) {
    body.open_struct();
    body.str(key);
    body.variant(sig);
    write(body);
    body.close_struct();
}

/// One `a{sa{sv}}` entry: the interface, then the properties `props`
/// makes.
pub fn iface(body: &mut Writer, name: &str, props: &dyn Fn(&mut Writer)) {
    body.open_struct();
    body.str(name);
    if let Some(cookie) = body.open_array(8) {
        props(body);
        body.close_array(cookie);
    }
    body.close_struct();
}

/// The `Adapter1` properties the bar reads (`None` omits), with the ones
/// it must skip beside them.
pub fn adapter_props(body: &mut Writer, powered: Option<bool>) {
    if let Some(powered) = powered {
        entry(body, "Powered", "b", &|w| w.boolean(powered));
    }
    entry(body, "Address", "s", &|w| {
        w.str("AA:BB:CC:DD:EE:FF");
    });
    entry(body, "Discovering", "b", &|w| w.boolean(false));
}

/// The `Device1` properties the bar reads (`None` omits), with the ones
/// it must skip beside them.
pub fn device_props(
    body: &mut Writer,
    connected: Option<bool>,
    name: Option<&str>,
    alias: Option<&str>,
) {
    if let Some(connected) = connected {
        entry(body, "Connected", "b", &|w| w.boolean(connected));
    }
    if let Some(name) = name {
        entry(body, "Name", "s", &|w| w.str(name));
    }
    if let Some(alias) = alias {
        entry(body, "Alias", "s", &|w| w.str(alias));
    }
    entry(body, "Address", "s", &|w| w.str("11:22:33:44:55:66"));
    entry(body, "RSSI", "n", &|w| w.i16(-60));
}

/// The `Battery1` properties the bar reads (`None` omits).
pub fn battery_props(body: &mut Writer, percentage: Option<u8>) {
    if let Some(percentage) = percentage {
        entry(body, "Percentage", "y", &|w| w.u8(percentage));
    }
}

/// One object of a `GetManagedObjects` answer: its path, then the
/// interfaces `ifaces` makes.
pub fn object(body: &mut Writer, path: &str, ifaces: &dyn Fn(&mut Writer)) {
    body.open_struct();
    body.str(path);
    if let Some(cookie) = body.open_array(8) {
        ifaces(body);
        body.close_array(cookie);
    }
    body.close_struct();
}

/// An adapter object: `Adapter1` (powered as given) under `path`.
pub fn adapter_object(body: &mut Writer, path: &str, powered: bool) {
    object(body, path, &|b| {
        iface(b, ADAPTER, &|p| adapter_props(p, Some(powered)));
    });
}

/// A device object: `Device1` (connected, named and aliased as given)
/// under `path`, with `Battery1` when `percentage` is `Some`.
pub fn device_object(
    body: &mut Writer,
    path: &str,
    connected: bool,
    name: Option<&str>,
    alias: Option<&str>,
    percentage: Option<u8>,
) {
    object(body, path, &|b| {
        iface(b, DEVICE, &|p| {
            device_props(p, Some(connected), name, alias)
        });
        if percentage.is_some() {
            iface(b, BATTERY, &|p| battery_props(p, percentage));
        }
    });
}

/// A `GetManagedObjects` answer body: `objects` fills the object array.
pub fn managed(objects: &dyn Fn(&mut Writer)) -> Vec<u8> {
    let mut body = Writer::new();
    if let Some(cookie) = body.open_array(8) {
        objects(&mut body);
        body.close_array(cookie);
    }
    body.take_body().unwrap_or_default()
}

/// A `GetManagedObjects` answer past the 1 MiB the client reads: what a
/// machine with very many known devices answers, and what the module
/// must skip whole. (`Writer::new` caps at what the client reads, so
/// this one is built uncapped.)
pub fn huge(objects: &dyn Fn(&mut Writer)) -> Vec<u8> {
    let mut body = Writer::with_cap(8 << 20);
    if let Some(cookie) = body.open_array(8) {
        objects(&mut body);
        body.close_array(cookie);
    }
    body.take_body().unwrap_or_default()
}

/// One adapter, powered, and one connected device with a battery: the
/// small world most tests start from.
pub fn small_world() -> Vec<u8> {
    managed(&|body| {
        adapter_object(body, "/org/bluez/hci0", true);
        device_object(
            body,
            "/org/bluez/hci0/dev_11_22_33_44_55_66",
            true,
            Some("Headset"),
            Some("Headset"),
            Some(72),
        );
    })
}

/// A `PropertiesChanged` body for `interface`: `entries` fills the
/// changed dictionary, `invalidated` names the properties that changed
/// with no value.
pub fn changed(interface: &str, entries: &dyn Fn(&mut Writer), invalidated: &[&str]) -> Vec<u8> {
    let mut body = Writer::new();
    body.str(interface);
    if let Some(cookie) = body.open_array(8) {
        entries(&mut body);
        body.close_array(cookie);
    }
    if let Some(cookie) = body.open_array(4) {
        for name in invalidated {
            body.str(name);
        }
        body.close_array(cookie);
    }
    body.take_body().unwrap_or_default()
}

/// An `InterfacesAdded` body: `object`'s path, then the interfaces
/// `ifaces` makes.
pub fn added(path: &str, ifaces: &dyn Fn(&mut Writer)) -> Vec<u8> {
    let mut body = Writer::new();
    body.str(path);
    if let Some(cookie) = body.open_array(8) {
        ifaces(&mut body);
        body.close_array(cookie);
    }
    body.take_body().unwrap_or_default()
}

/// An `InterfacesRemoved` body: the path and the lost interface names.
pub fn removed(path: &str, names: &[&str]) -> Vec<u8> {
    let mut body = Writer::new();
    body.str(path);
    if let Some(cookie) = body.open_array(4) {
        for name in names {
            body.str(name);
        }
        body.close_array(cookie);
    }
    body.take_body().unwrap_or_default()
}

/// A `GetAll` answer body for `Device1`: `entries` fills the dictionary.
pub fn get_all(entries: &dyn Fn(&mut Writer)) -> Vec<u8> {
    let mut body = Writer::new();
    if let Some(cookie) = body.open_array(8) {
        entries(&mut body);
        body.close_array(cookie);
    }
    body.take_body().unwrap_or_default()
}
