#!/usr/bin/env perl
# PNG region probe: prints nonblack count and md5 of the region.
# usage: probe.pl FILE X Y W H
use strict;
use warnings;
use Compress::Zlib;
use Digest::MD5 qw(md5_hex);

my ($file, $x, $y, $w, $h) = @ARGV;
open my $fh, '<:raw', $file or die "open: $!";
my $data = do { local $/; <$fh> };
close $fh;
substr($data, 0, 8) eq "\x89PNG\r\n\x1a\n" or die "not png";

my ($width, $height, $ctype, $depth);
my $idat = '';
my $pos = 8;
while ($pos < length($data)) {
    my ($len, $type) = unpack('N a4', substr($data, $pos, 8));
    my $chunk = substr($data, $pos + 8, $len);
    if ($type eq 'IHDR') {
        ($width, $height, $depth, $ctype) = unpack('N N C C', $chunk);
    } elsif ($type eq 'IDAT') {
        $idat .= $chunk;
    } elsif ($type eq 'IEND') {
        last;
    }
    $pos += 12 + $len;
}
die "need 8-bit rgb/rgba, got depth=$depth ctype=$ctype"
    unless $depth == 8 && ($ctype == 2 || $ctype == 6);
my $bpp = $ctype == 2 ? 3 : 4;
my $raw = uncompress($idat) or die "uncompress";
my $stride = $width * $bpp;
my @rows;
my $p = 0;
my @prev = (0) x $stride;
for my $r (0 .. $height - 1) {
    my $f = ord(substr($raw, $p, 1)); $p++;
    my @cur = unpack('C*', substr($raw, $p, $stride)); $p += $stride;
    for my $i (0 .. $stride - 1) {
        my $a = $i >= $bpp ? $cur[$i - $bpp] : 0;
        my $b = $prev[$i];
        my $c = $i >= $bpp ? $prev[$i - $bpp] : 0;
        my $pred = 0;
        if ($f == 1) { $pred = $a; }
        elsif ($f == 2) { $pred = $b; }
        elsif ($f == 3) { $pred = int(($a + $b) / 2); }
        elsif ($f == 4) {
            my $p = $a + $b - $c;
            my ($pa, $pb, $pc) = (abs($p - $a), abs($p - $b), abs($p - $c));
            $pred = ($pa <= $pb && $pa <= $pc) ? $a : ($pb <= $pc ? $b : $c);
        }
        $cur[$i] = ($cur[$i] + $pred) & 0xff;
    }
    push @rows, \@cur;
    @prev = @cur;
}
my ($nonbg, $bright, $bytes) = (0, 0, '');
for my $ry ($y .. $y + $h - 1) {
    last if $ry >= $height;
    for my $rx ($x .. $x + $w - 1) {
        last if $rx >= $width;
        my $o = $rx * $bpp;
        my ($r, $g, $b) = @{$rows[$ry]}[$o, $o + 1, $o + 2];
        $bytes .= pack('CCC', $r, $g, $b);
        $nonbg++ if abs($r - 20) + abs($g - 20) + abs($b - 25) > 24;
        $bright++ if $r + $g + $b > 400;
    }
}
print "nonbg=$nonbg bright=$bright md5=" . md5_hex($bytes) . "\n";
