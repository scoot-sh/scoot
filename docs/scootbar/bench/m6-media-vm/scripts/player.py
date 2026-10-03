# An MPRIS player written against jeepney (an independent D-Bus marshaller,
# not scootbar's): owns org.mpris.MediaPlayer2.NAME, answers
# Properties.GetAll/Get for the Player interface, handles PlayPause, Next and
# Previous (logged, and a changed state is signalled as PropertiesChanged),
# and does what flag files ask: FLAG.track changes the track, FLAG.die exits
# without releasing anything (the connection drops), FLAG.long makes the
# track the long title, FLAG.flood starts
# signalling Position as fast as it can for 20 s, FLAG.metaflood the same
# with a different title each time, FLAG.flapflood alternating the status
# between Playing and Paused each time.
# usage: player.py NAME LOG FLAG [Playing|Paused|Stopped]
import os, sys, time
import jeepney
from jeepney import DBusAddress, new_method_return, new_error, new_signal, MessageType, HeaderFields
from jeepney.io.blocking import open_dbus_connection
from jeepney.bus_messages import message_bus

name, log, flag = sys.argv[1], sys.argv[2], sys.argv[3]
status = sys.argv[4] if len(sys.argv) > 4 else 'Playing'
PATH = '/org/mpris/MediaPlayer2'
PLAYER = 'org.mpris.MediaPlayer2.Player'
tracks = [('Track One', ['Ada Lovelace']), ('Track Two', ['Bo', 'Cy']), ('A very long title that goes on and on for far more than a bar can show in one go', ['An Artist With A Long Name'])]
track = 0

def say(text):
    with open(log, 'a') as f:
        f.write(text + '\n')

def metadata():
    title, artists = tracks[track % len(tracks)]
    return ('a{sv}', {
        'mpris:trackid': ('o', '/org/mpris/track/%d' % track),
        'mpris:length': ('x', 215000000),
        'mpris:artUrl': ('s', 'file:///tmp/cover.jpg'),
        'xesam:title': ('s', title),
        'xesam:artist': ('as', artists),
        'xesam:album': ('s', 'An Album'),
    })

def props():
    return {
        'PlaybackStatus': ('s', status),
        'Metadata': metadata(),
        'Position': ('x', 42000000),
        'Volume': ('d', 0.5),
        'Rate': ('d', 1.0),
        'CanGoNext': ('b', True),
        'CanGoPrevious': ('b', True),
        'CanPlay': ('b', True),
        'CanPause': ('b', True),
        'CanControl': ('b', True),
    }

conn = open_dbus_connection(bus='SESSION')
reply = conn.send_and_get_reply(message_bus.RequestName(name))
say('requestname %r' % (reply.body,))
me = DBusAddress(PATH, bus_name=name, interface='org.freedesktop.DBus.Properties')

def changed(entries):
    conn.send(new_signal(me, 'PropertiesChanged', 'sa{sv}as', (PLAYER, entries, [])))

say('ready')
flood = None
n = 0
while True:
    try:
        msg = conn.receive(timeout=0.0005 if flood else 0.3)
    except TimeoutError:
        msg = None
    if flood:
        n += 1
        if flood == 'pos':
            changed({'Position': ('x', n)})
        elif flood == 'flap':
            status = 'Paused' if status == 'Playing' else 'Playing'
            changed({'PlaybackStatus': ('s', status)})
        else:
            tracks[0] = ('Flood %d' % n, ['Flooder'])
            track = 0
            changed({'Metadata': metadata()})
        if time.time() > flood_end:
            say('flood sent %d' % n)
            flood = None
    for suffix in ('track', 'long', 'die', 'flood', 'metaflood', 'flapflood'):
        path = '%s.%s' % (flag, suffix)
        if os.path.exists(path):
            os.unlink(path)
            if suffix == 'track':
                track += 1
                changed({'Metadata': metadata()})
                say('track %d' % track)
            elif suffix == 'long':
                track = 2
                changed({'Metadata': metadata()})
                say('track %d (long)' % track)
            elif suffix == 'die':
                say('die')
                os._exit(0)
            else:
                flood = {'flood': 'pos', 'metaflood': 'meta', 'flapflood': 'flap'}[suffix]
                flood_end = time.time() + 20
                n = 0
                say('flood start %s' % flood)
    if msg is None:
        continue
    h = msg.header
    if h.message_type != MessageType.method_call:
        continue
    member = h.fields.get(HeaderFields.member)
    iface = h.fields.get(HeaderFields.interface)
    if member == 'GetAll':
        say('getall')
        conn.send(new_method_return(msg, 'a{sv}', (props(),)))
    elif member == 'Get':
        which = msg.body[1] if len(msg.body) > 1 else ''
        value = props().get(which)
        say('get %s' % which)
        if value is None:
            conn.send(new_error(msg, 'org.freedesktop.DBus.Error.UnknownProperty'))
        else:
            conn.send(new_method_return(msg, 'v', (value,)))
    elif iface == PLAYER and member in ('PlayPause', 'Next', 'Previous'):
        say(member)
        conn.send(new_method_return(msg))
        if member == 'PlayPause':
            status = 'Paused' if status == 'Playing' else 'Playing'
            changed({'PlaybackStatus': ('s', status)})
        else:
            track += 1 if member == 'Next' else -1
            changed({'Metadata': metadata()})
    elif member == 'Ping':
        conn.send(new_method_return(msg))
    else:
        conn.send(new_error(msg, 'org.freedesktop.DBus.Error.UnknownMethod'))
