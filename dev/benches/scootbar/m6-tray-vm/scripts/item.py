# A StatusNotifierItem written against jeepney (an independent D-Bus
# marshaller, not scootbar's): owns an item name, registers it with the
# watcher, answers Properties.GetAll with a pixmap, logs Activate/Scroll.
import os, sys, struct
from jeepney import DBusAddress, new_method_call, new_method_return, new_error, new_signal, MessageType
from jeepney.io.blocking import open_dbus_connection
from jeepney.bus_messages import message_bus

name, log, flag = sys.argv[1], sys.argv[2], sys.argv[3]
shade = int(sys.argv[4]) if len(sys.argv) > 4 else 0
register = (len(sys.argv) <= 5) or sys.argv[5] != "noregister"
flood = len(sys.argv) > 6 and sys.argv[6] == "flood"
PATH = '/StatusNotifierItem'

def pixmap(w, h, r, g, b):
    return (w, h, b''.join(bytes([255, r, g, b]) for _ in range(w * h)))

def props(v):
    return {
        'Category': ('s', 'ApplicationStatus'),
        'Id': ('s', name),
        'Title': ('s', 'item-' + str(v)),
        'Status': ('s', 'Active'),
        'IconPixmap': ('a(iiay)', [pixmap(22, 22, 200 - v * 60, 40 + shade, 60 + v * 60), pixmap(16, 16, 200 - v * 60, 40 + shade, 60 + v * 60)]),
        'AttentionIconPixmap': ('a(iiay)', []),
        'ToolTip': ('(sa(iiay)ss)', ('', [], 'tip-' + str(v), 'tooltext')),
        'ItemIsMenu': ('b', False),
        'Menu': ('o', '/NO_DBUSMENU'),
        'WindowId': ('u', 0),
        'IconName': ('s', ''),
    }

def say(text):
    with open(log, 'a') as f:
        f.write(text + '\n')

conn = open_dbus_connection(bus='SESSION')
reply = conn.send_and_get_reply(message_bus.RequestName(name))
say('requestname %r' % (reply.body,))
if register:
    w = DBusAddress('/StatusNotifierWatcher', bus_name='org.kde.StatusNotifierWatcher', interface='org.kde.StatusNotifierWatcher')
    r = conn.send_and_get_reply(new_method_call(w, 'RegisterStatusNotifierItem', 's', (name,)))
    say('register %r' % (r.body,))
version = 0
me = DBusAddress(PATH, bus_name=name, interface='org.kde.StatusNotifierItem')
say('ready')
while True:
    try:
        msg = conn.receive(timeout=0.3)
    except TimeoutError:
        if os.path.exists(flag):
            os.unlink(flag)
            version = (version + 1) % 3
            conn.send(new_signal(me, 'NewIcon'))
            say('newicon %d' % version)
        continue
    h = msg.header
    if h.message_type != MessageType.method_call:
        continue
    member = h.fields.get(__import__('jeepney').HeaderFields.member)
    if member == 'GetAll':
        conn.send(new_method_return(msg, 'a{sv}', (props(version),)))
        say('getall -> v%d' % version)
        if flood:
            version = (version + 1) % 3
            conn.send(new_signal(me, 'NewIcon'))
    elif member in ('Activate', 'SecondaryActivate', 'Scroll'):
        say('%s %r' % (member, msg.body))
        conn.send(new_method_return(msg))
    else:
        conn.send(new_error(msg, 'org.freedesktop.DBus.Error.UnknownMethod'))
