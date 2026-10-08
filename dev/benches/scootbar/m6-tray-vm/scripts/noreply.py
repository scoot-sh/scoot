import sys
from jeepney.io.blocking import open_dbus_connection
from jeepney.bus_messages import message_bus
conn = open_dbus_connection(bus='SESSION')
r = conn.send_and_get_reply(message_bus.RequestName('sh.scoot.NoReply'))
print('owner ready', r.body, flush=True)
while True:
    try:
        conn.receive(timeout=1.0)   # receive and never reply
    except TimeoutError:
        pass
