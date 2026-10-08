#!/usr/bin/perl
# A scripted BlueZ for the bluetooth bench: owns org.bluez on a private
# daemon, answers GetManagedObjects and GetAll from fixture files (marshalled
# by scootbar's own test builders), prints Set(Powered), and optionally
# floods PropertiesChanged. An independent D-Bus speaker (raw socket), so
# the bar is measured against a second implementation, not itself.
use strict;
use warnings;
use IO::Socket::UNIX;
use File::Basename;
$| = 1;
# Fixture bodies beside the script (marshalled by scootbar's own test
# builders: `src/dbus/bluez/build.rs`'s `small_world` and `get_all`s).
my $FIX = dirname(__FILE__) . '/fixtures';

my ($sockpath, $mode, $count) = @ARGV;
die "usage: $0 SOCKET [idle|flood N]\n" unless $sockpath;
$mode //= 'idle';

sub slurp { open(my $f, '<:raw', $_[0]) or die "slurp $_[0]: $!"; local $/; my $d = <$f>; $d }
my $managed = slurp("$FIX/bt-managed.bin");
my %getall = (
    "/org/bluez/hci0\x00org.bluez.Adapter1" => slurp("$FIX/bt-getall-adapter.bin"),
    "/org/bluez/hci0/dev_11_22_33_44_55_66\x00org.bluez.Device1" => slurp("$FIX/bt-getall-device.bin"),
    "/org/bluez/hci0/dev_11_22_33_44_55_66\x00org.bluez.Battery1" => slurp("$FIX/bt-getall-battery.bin"),
);
my $changed_body = slurp("$FIX/bt-changed.bin");

my $sock = IO::Socket::UNIX->new(Peer => $sockpath) or die "connect: $!";
$sock->autoflush(1);
syswrite($sock, "\0AUTH EXTERNAL " . unpack('H*', "$<") . "\r\n") or die;
expect("OK");
syswrite($sock, "BEGIN\r\n") or die;

my $serial = 1;
send_call('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'Hello', '', '');
my $fr = read_return();
die "no hello reply" unless $fr->{type} == 2;
my $unique = get_string($fr->{body});
print "peer unique: $unique\n";
my $b = marshal_str('org.bluez');
marshal_u32(\$b, 0);
send_call('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName', 'su', $b);
$fr = read_return();
die "no primary" unless unpack('L<', $fr->{body}) == 1;
print "owns org.bluez\n";

if ($mode eq 'flood') {
    $count //= 10000;
    sleep(2);  # let the bar enumerate first
    my $sig = frame_signal('/org/bluez/hci0/dev_11_22_33_44_55_66',
        'org.freedesktop.DBus.Properties', 'PropertiesChanged', $unique, 'sa{sv}as', $changed_body);
    my $t0 = time();
    for (1 .. $count) { syswrite($sock, $sig) or die "flood: $!"; }
    printf "flooded %d in %ds\n", $count, time() - $t0;
}

while (1) {
    $fr = read_frame();
    next unless $fr->{type} == 1;
    my %f = parse_fields($fr->{fields});
    my $sender = $f{7} // '';
    my $member = $f{3} // '';
    if ($member eq 'AddMatch') {
        send_return($fr->{serial}, $sender, '', '');
    } elsif ($member eq 'GetManagedObjects') {
        send_return($fr->{serial}, $sender, 'a{oa{sa{sv}}}', $managed);
    } elsif ($member eq 'GetAll') {
        my $key = ($f{1} // '') . "\x00" . get_string($fr->{body});
        if (exists $getall{$key}) {
            send_return($fr->{serial}, $sender, 'a{sv}', $getall{$key});
        } else {
            send_error($fr->{serial}, $sender, 'org.freedesktop.DBus.Error.UnknownMethod');
        }
    } elsif ($member eq 'Set') {
        my ($iface, $prop, $val) = parse_set($fr->{body});
        print "SET $iface.$prop = $val\n";
    }
}

sub expect {
    my ($want) = @_;
    my $line = '';
    while ($line !~ /\r\n$/) { sysread($sock, $line, 1, length($line)) or die "auth: $!"; }
    die "auth: got $line" unless $line =~ /^\Q$want\E/;
}

sub read_exact {
    my ($n) = @_;
    my $buf = '';
    while (length($buf) < $n) {
        my $r = sysread($sock, $buf, $n - length($buf), length($buf));
        die "eof" unless $r;
    }
    $buf
}

sub read_return {
    while (1) {
        my $fr = read_frame();
        return $fr if $fr->{type} == 2;
    }
}

sub read_frame {
    my $h = read_exact(16);
    my ($endian, $type, $flags, $ver, $bodylen, $rserial, $fieldslen) =
        unpack('CCCCL<L<L<', $h);
    die "big endian ($endian)" unless $endian == ord('l');
    my $rest = read_exact((($fieldslen + 7) & ~7) + $bodylen);
    return {
        type => $type,
        serial => $rserial,
        fields => substr($rest, 0, $fieldslen),
        body => substr($rest, ($fieldslen + 7) & ~7),
    };
}

# Header fields into {code => string} for s/o values (all this peer reads).
sub parse_fields {
    my ($f) = @_;
    my %out;
    my $off = 0;
    while ($off < length($f)) {
        $off += (8 - $off % 8) % 8;
        last if $off >= length($f);
        my $code = unpack('C', substr($f, $off, 1)); $off += 1;
        my $siglen = unpack('C', substr($f, $off, 1)); $off += 1;
        my $sig = substr($f, $off, $siglen); $off += $siglen + 1;
        if ($sig eq 's' || $sig eq 'o' || $sig eq 'u') {
            $off += (4 - $off % 4) % 4;
            if ($sig eq 'u') {
                $out{$code} = unpack('L<', substr($f, $off, 4));
                $off += 4;
            } else {
                my $len = unpack('L<', substr($f, $off, 4));
                $out{$code} = substr($f, $off + 4, $len);
                $off += 4 + $len + 1;
            }
        } elsif ($sig eq 'g') {
            # The body's signature: not routed on, skipped.
            my $len = unpack('C', substr($f, $off, 1));
            $off += 1 + $len + 1;
        } else {
            die "field sig $sig";
        }
    }
    %out
}

sub get_string {
    my ($buf) = @_;
    my $len = unpack('L<', substr($buf, 0, 4));
    substr($buf, 4, $len)
}

# Set body: interface, property, then a variant holding a bool.
sub parse_set {
    my ($buf) = @_;
    my $off = 0;
    my @str;
    for (1 .. 2) {
        my $len = unpack('L<', substr($buf, $off, 4));
        push @str, substr($buf, $off + 4, $len);
        $off += 4 + $len + 1;
    }
    my $siglen = unpack('C', substr($buf, $off, 1)); $off += 1;
    my $sig = substr($buf, $off, $siglen); $off += $siglen + 1;
    die "set sig $sig" unless $sig eq 'b';
    $off += (4 - $off % 4) % 4;
    my $val = unpack('L<', substr($buf, $off, 4)) ? 'true' : 'false';
    (@str, $val)
}

sub marshal_str { my ($s) = @_; pack('L<', length($s)) . $s . "\0" }

# One header field entry (unpadded); field_array pads between entries.
sub field_entry {
    my ($code, $sig, $marshalled) = @_;
    pack('C', $code) . pack('C', length($sig)) . $sig . "\0" . $marshalled
}

sub field_array {
    my $f = '';
    for my $e (@_) { $f .= "\0" x ((8 - length($f) % 8) % 8); $f .= $e; }
    $f
}

sub frame {
    my ($type, $rserial, $fields, $body, $flags) = @_;
    $flags //= 0;
    # No end padding: the length is the body's own (the Rust writer is
    # the reference: 18 bytes of string+u32 ride as 20 with the u32's
    # own alignment, and nothing after).
    my $h = pack('CCCC', ord('l'), $type, $flags, 1) . pack('L<', length($body))
        . pack('L<', $rserial) . pack('L<', length($fields)) . $fields;
    $h .= "\0" x ((8 - length($h) % 8) % 8);
    $h . $body
}

# A u32 after whatever is built so far, aligned as the wire needs.
sub marshal_u32 {
    my ($bref, $v) = @_;
    $$bref .= "\0" x ((4 - length($$bref) % 4) % 4);
    $$bref .= pack('L<', $v);
}

sub send_call {
    my ($dest, $path, $iface, $member, $sig, $body) = @_;
    my $f = field_array(
        field_entry(1, 'o', marshal_str($path)),
        field_entry(2, 's', marshal_str($iface)),
        field_entry(3, 's', marshal_str($member)),
        field_entry(6, 's', marshal_str($dest)),
        field_entry(8, 'g', pack('C', length($sig)) . $sig . "\0"));
    my $fr = frame(1, $serial++, $f, $body);
    syswrite($sock, $fr) or die;
}

sub send_return {
    my ($to, $dest, $sig, $body) = @_;
    my $f = field_array(
        field_entry(5, 'u', pack('L<', $to)),
        field_entry(6, 's', marshal_str($dest)),
        field_entry(8, 'g', pack('C', length($sig)) . $sig . "\0"));
    syswrite($sock, frame(2, $serial++, $f, $body)) or die;
}

sub send_error {
    my ($to, $dest, $name) = @_;
    my $f = field_array(
        field_entry(4, 's', marshal_str($name)),
        field_entry(5, 'u', pack('L<', $to)),
        field_entry(6, 's', marshal_str($dest)));
    syswrite($sock, frame(3, $serial++, $f, '')) or die;
}

sub frame_signal {
    my ($path, $iface, $member, $sender, $sig, $body) = @_;
    my $f = field_array(
        field_entry(1, 'o', marshal_str($path)),
        field_entry(2, 's', marshal_str($iface)),
        field_entry(3, 's', marshal_str($member)),
        field_entry(7, 's', marshal_str($sender)),
        field_entry(8, 'g', pack('C', length($sig)) . $sig . "\0"));
    frame(4, $serial++, $f, $body)
}
