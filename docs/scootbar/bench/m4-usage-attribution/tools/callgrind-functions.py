import sys,re,collections
def fnset(p,exe='scootbar'):
    names={};obs={};ob=None;res=set()
    for l in open(p):
        l=l.rstrip('\n')
        for pre,store in (('ob=',None),('cob=',None)):
            if l.startswith(pre):
                m=re.match(pre+r'\((\d+)\)\s*(.*)',l)
                if m.group(2): obs[m.group(1)]=m.group(2)
                if pre=='ob=': ob=obs[m.group(1)]
        if l.startswith('fn=') :
            m=re.match(r'fn=\((\d+)\)\s*(.*)',l)
            if m.group(2): names[m.group(1)]=m.group(2)
            if ob and ob.endswith(exe): res.add(re.sub(r"'\d+$",'',names[m.group(1)]))
        elif l.startswith('cfn='):
            m=re.match(r'cfn=\((\d+)\)\s*(.*)',l)
            if m.group(2): names[m.group(1)]=m.group(2)
    return res
n=sys.argv[1]
syms={}
for l in open(f'nml-{n}.txt'):
    t=l.split()
    if len(t)==4 and t[2] in 'tTwW': syms[t[3]]=(int(t[0],16),int(t[1],16))
hot=fnset(f'cgm-{n}.out')
miss=[h for h in hot if h not in syms]
tot=sum(syms[h][1] for h in hot if h in syms)
P=16384
pages=set();w=set()
for h in hot:
    if h in syms:
        a,s=syms[h]
        for p in range(a//P,(a+max(s,1)-1)//P+1): pages.add(p)
        for p in range(a//65536,(a+max(s,1)-1)//65536+1): w.add(p)
print(n,'hot fns',len(hot),'missing in nm',len(miss),'bytes',tot,'16K pages',len(pages),'64K windows',len(w))
open(f'hot-{n}.txt','w').write('\n'.join(sorted(h for h in hot if h in syms))+'\n')
