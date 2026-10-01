import sys,struct,re
n=sys.argv[1]; d=sys.argv[2]
pm=open(f'q/{d}/text.pagemap','rb').read()
pres=[(struct.unpack_from('<Q',pm,i*8)[0]>>63)&1 for i in range(len(pm)//8)]
secs=[]
for l in open(f'sec-{n}.txt'):
    m=re.match(r'\s*\[\s*\d+\]\s+(\S+)\s+\S+\s+([0-9a-f]+)\s+[0-9a-f]+\s+([0-9a-f]+)',l)
    if m and int(m.group(2),16)<0x200000: secs.append((m.group(1),int(m.group(2),16),int(m.group(3),16)))
print(n,d,'pages',len(pres),'resident',sum(pres))
P=16384
for name,a,s in secs:
    if s==0: continue
    p0=a//P; p1=(a+s-1)//P
    tot=p1-p0+1; r=sum(pres[p0:p1+1])
    print(f'  {name:18} {a:#8x} +{s//1024:5}k pages {p0}-{p1}: resident {r}/{tot}')
print(''.join(str(x) for x in pres))
