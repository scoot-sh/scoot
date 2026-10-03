//! The BlueZ shapes without a bus: what each reader takes and refuses.

use super::build::{self, entry};
use super::{ADAPTER, BATTERY, DEVICE, Iface, read_interfaces_added, read_interfaces_removed};
use super::{MAX_INTERFACES, read_properties_changed};
use super::{read_adapter_all, read_battery_all, read_device_all, read_managed_objects};
use crate::dbus::proto::Writer;

#[test]
fn a_small_world_reads_whole() {
    let body = build::small_world();
    let objects = read_managed_objects(&body).unwrap();
    assert_eq!(objects.len(), 2);
    let adapter = &objects[0];
    assert_eq!(adapter.path, "/org/bluez/hci0");
    assert_eq!(adapter.adapter.as_ref().unwrap().powered, Some(true));
    assert!(adapter.device.is_none());
    let device = &objects[1];
    assert_eq!(device.path, "/org/bluez/hci0/dev_11_22_33_44_55_66");
    let props = device.device.as_ref().unwrap();
    assert_eq!(props.connected, Some(true));
    assert_eq!(props.name, Some("Headset"));
    assert_eq!(props.alias, Some("Headset"));
    assert_eq!(device.battery.as_ref().unwrap().percentage, Some(72));
}

#[test]
fn an_empty_world_reads_empty() {
    let body = build::managed(&|_| {});
    assert_eq!(read_managed_objects(&body).unwrap(), vec![]);
}

#[test]
fn unknown_interfaces_are_walked_past() {
    let body = build::managed(&|body| {
        build::object(body, "/org/bluez/hci0", &|b| {
            build::iface(b, "org.freedesktop.DBus.Properties", &|_| {});
            build::iface(b, "org.bluez.GattService1", &|p| {
                entry(p, "UUID", "s", &|w| w.str("abc"));
            });
        });
    });
    let objects = read_managed_objects(&body).unwrap();
    assert_eq!(objects.len(), 1);
    assert!(objects[0].adapter.is_none());
    assert!(objects[0].device.is_none());
}

#[test]
fn a_property_of_the_wrong_type_loses_only_itself() {
    let body = build::managed(&|body| {
        build::adapter_object(body, "/org/bluez/hci0", true);
        build::object(body, "/org/bluez/hci0/dev_00", &|b| {
            build::iface(b, DEVICE, &|p| {
                entry(p, "Connected", "s", &|w| w.str("yes"));
                entry(p, "Alias", "s", &|w| w.str("Kept"));
            });
        });
    });
    let objects = read_managed_objects(&body).unwrap();
    assert_eq!(objects.len(), 2);
    let device = objects[1].device.as_ref().unwrap();
    assert_eq!(device.connected, None);
    assert_eq!(device.alias, Some("Kept"));
}

#[test]
fn too_many_interfaces_refuse_the_object() {
    let body = build::managed(&|body| {
        build::object(body, "/org/bluez/hci0", &|b| {
            for n in 0..MAX_INTERFACES + 1 {
                build::iface(b, &format!("org.example.Iface{n}"), &|_| {});
            }
        });
    });
    assert!(read_managed_objects(&body).is_err());
}

#[test]
fn too_many_properties_refuse_the_interface() {
    let mut body = Writer::new();
    if let Some(cookie) = body.open_array(8) {
        build::object(&mut body, "/org/bluez/hci0", &|b| {
            build::iface(b, ADAPTER, &|p| {
                for n in 0..200 {
                    entry(p, &format!("Prop{n}"), "s", &|w| w.str("x"));
                }
            });
        });
        body.close_array(cookie);
    }
    let body = body.take_body().unwrap();
    assert!(read_managed_objects(&body).is_err());
}

#[test]
fn trailing_bytes_refuse_the_answer() {
    let mut body = build::small_world();
    body.push(0);
    assert!(read_managed_objects(&body).is_err());
}

#[test]
fn a_bad_object_path_refuses_the_answer() {
    let body = build::managed(&|body| {
        build::object(body, "not-a-path", &|_| {});
    });
    assert!(read_managed_objects(&body).is_err());
}

#[test]
fn getall_bodies_read_each_interface() {
    let adapter = build::get_all(&|p| build::adapter_props(p, Some(false)));
    assert_eq!(read_adapter_all(&adapter).unwrap().powered, Some(false));
    let device = build::get_all(&|p| build::device_props(p, Some(true), None, Some("A")));
    let props = read_device_all(&device).unwrap();
    assert_eq!(props.connected, Some(true));
    assert_eq!(props.name, None);
    assert_eq!(props.alias, Some("A"));
    let battery = build::get_all(&|p| build::battery_props(p, Some(100)));
    assert_eq!(read_battery_all(&battery).unwrap().percentage, Some(100));
    // A percentage is a byte: a wider type is not one.
    let wide = build::get_all(&|p| entry(p, "Percentage", "q", &|w| w.u16(72)));
    assert_eq!(read_battery_all(&wide).unwrap().percentage, None);
}

#[test]
fn property_signals_read_their_interface() {
    let body = build::changed(
        DEVICE,
        &|p| entry(p, "Connected", "b", &|w| w.boolean(false)),
        &[],
    );
    let changed = read_properties_changed(&body).unwrap();
    assert_eq!(changed.iface, Iface::Device);
    assert_eq!(changed.device.connected, Some(false));
    assert!(!changed.invalidated);

    let body = build::changed(ADAPTER, &|_| {}, &["Powered"]);
    let changed = read_properties_changed(&body).unwrap();
    assert_eq!(changed.iface, Iface::Adapter);
    assert_eq!(changed.adapter.powered, None);
    assert!(changed.invalidated);

    let body = build::changed("org.bluez.GattService1", &|_| {}, &["UUID"]);
    let changed = read_properties_changed(&body).unwrap();
    assert_eq!(changed.iface, Iface::Other);
    assert!(!changed.invalidated);
}

#[test]
fn interfaces_added_and_removed_read() {
    let body = build::added("/org/bluez/hci0/dev_01", &|b| {
        build::iface(b, DEVICE, &|p| {
            build::device_props(p, Some(false), None, Some("New"));
        });
    });
    let object = read_interfaces_added(&body).unwrap();
    assert_eq!(object.path, "/org/bluez/hci0/dev_01");
    assert_eq!(object.device.as_ref().unwrap().alias, Some("New"));

    let body = build::removed("/org/bluez/hci0/dev_01", &[DEVICE, BATTERY]);
    let (path, names) = read_interfaces_removed(&body).unwrap();
    assert_eq!(path, "/org/bluez/hci0/dev_01");
    assert_eq!(names, vec![DEVICE, BATTERY]);
}
