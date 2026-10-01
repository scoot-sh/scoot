#!/usr/bin/env bash
# evidence.sh -- after the benches: attribution quick runs of the benchmarked binaries, three each, into ~/code/m4u/attr
cd ~/code/m4u
rm -rf attr; mkdir -p attr
S=$PWD/t-scoot/release/scoot
for n in base tip cand; do
  for i in 1 2 3; do ./quick.sh $S $PWD/t-$n/release/scootbar attr/$n$i 20 >/dev/null 2>&1; done
done
for n in base tip cand; do for i in 1 2 3; do d=attr/$n$i; echo $n$i $(grep -E "^(Rss|Pss|Pss_Anon|Pss_File|Pss_Shmem):" $d/rollup | tr -s " " | tr "\n" " ") text-Rss $(grep -A4 -E "r-xp.*scootbar$" $d/smaps | grep -E "^Rss" | tr -s " ") minflt $(cut -d" " -f10 $d/stat) early-minflt $(cut -d" " -f10 $d/stat.early); done; done
