import math, random
INK="#0b0b0c"; PAPER="#efe9dc"
class Svg:
    def __init__(s, bg, title):
        s.parts=[]; s.defs=[]; s.n=0; s.bg=bg; s.title=title
    def id(s):
        s.n+=1; return f"c{s.n}"
    def clip(s, d):
        i=s.id(); s.defs.append(f'<clipPath id="{i}"><path d="{d}"/></clipPath>'); return i
    def add(s, x): s.parts.append(x)
    def path(s, d, fill=PAPER, stroke=INK, w=7, extra=""):
        st=f' stroke="{stroke}" stroke-width="{w}" stroke-linejoin="round" stroke-linecap="round"' if stroke else ""
        s.add(f'<path d="{d}" fill="{fill}"{st} {extra}/>')
    def group(s, clip_ids, inner):
        out="".join(inner)
        for c in reversed(clip_ids): out=f'<g clip-path="url(#{c})">{out}</g>'
        s.add(out)
    def out(s):
        return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">'
          f'<title>{s.title}</title><defs><clipPath id="t"><rect width="512" height="512" rx="112"/></clipPath>{"".join(s.defs)}</defs>'
          f'<g clip-path="url(#t)"><rect width="512" height="512" fill="{s.bg}"/>{"".join(s.parts)}</g></svg>')

def hatch(angle, step, w, x0=-100, y0=-100, x1=612, y1=612, color=INK, jitter=0.0, seed=1):
    r=random.Random(seed); a=math.radians(angle); dx,dy=math.cos(a),math.sin(a); nx,ny=-dy,dx
    cx,cy=(x0+x1)/2,(y0+y1)/2; L=max(x1-x0,y1-y0)
    out=[]; k=-L
    while k<L:
        o=k+r.uniform(-jitter,jitter)
        px,py=cx+nx*o,cy+ny*o
        out.append(f'<line x1="{px-dx*L:.1f}" y1="{py-dy*L:.1f}" x2="{px+dx*L:.1f}" y2="{py+dy*L:.1f}" stroke="{color}" stroke-width="{w}"/>')
        k+=step
    return out

def curves(fn, n, w, color=INK, steps=40):
    """fn(t, u) -> (x, y): family of n curves u in [0,1], each sampled along t in [0,1]."""
    out=[]
    for i in range(n):
        u=i/(max(n-1,1))
        pts=[fn(j/steps,u) for j in range(steps+1)]
        d="M"+" L".join(f"{x:.1f} {y:.1f}" for x,y in pts)
        ww=w(u) if callable(w) else w
        out.append(f'<path d="{d}" fill="none" stroke="{color}" stroke-width="{ww:.2f}" stroke-linecap="round"/>')
    return out

def stipple(x0,y0,x1,y1,n,rmin,rmax,color=INK,seed=2):
    r=random.Random(seed)
    return [f'<circle cx="{r.uniform(x0,x1):.1f}" cy="{r.uniform(y0,y1):.1f}" r="{r.uniform(rmin,rmax):.2f}" fill="{color}"/>' for _ in range(n)]

def scallops(rows, color=INK, fill=PAPER, w=2.2, seed=3):
    """rows: list of (y, x0, x1, size). Feather scallops (U arcs) with a short inner stroke."""
    r=random.Random(seed); out=[]
    for k,(y,x0,x1,sz) in enumerate(rows):
        x=x0+(sz/2 if k%2 else 0)
        while x<x1:
            yy=y+r.uniform(-1.5,1.5)
            out.append(f'<path d="M{x-sz/2:.1f} {yy:.1f} Q{x:.1f} {yy+sz*0.95:.1f} {x+sz/2:.1f} {yy:.1f}" fill="{fill}" stroke="{color}" stroke-width="{w}"/>')
            out.append(f'<path d="M{x:.1f} {yy+sz*0.15:.1f} L{x+r.uniform(-1,1):.1f} {yy+sz*0.55:.1f}" stroke="{color}" stroke-width="{w*0.6:.2f}" stroke-linecap="round"/>')
            x+=sz
    return out

def arcs(cx,cy,radii,a0,a1,w,color=PAPER):
    out=[]
    for rr in radii:
        p0=(cx+rr*math.cos(math.radians(a0)), cy+rr*math.sin(math.radians(a0)))
        p1=(cx+rr*math.cos(math.radians(a1)), cy+rr*math.sin(math.radians(a1)))
        out.append(f'<path d="M{p0[0]:.1f} {p0[1]:.1f} A{rr} {rr} 0 0 1 {p1[0]:.1f} {p1[1]:.1f}" fill="none" stroke="{color}" stroke-width="{w}" stroke-linecap="round"/>')
    return out
