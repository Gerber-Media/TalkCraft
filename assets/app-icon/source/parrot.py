from lib import *
s=Svg("#2f9e63","TalkCraft")
# speech arcs from the open bill
s.add("".join(arcs(150,292,[46,74,102],152,208,9)))
head=("M214 176 C224 112 298 78 362 92 C432 108 474 168 474 236 C474 296 512 330 540 352 "
      "L540 540 L248 540 C254 470 250 404 240 352 C234 320 226 302 214 296 Z")
s.path(head,w=8)
hc=s.clip(head)
# feather scallops on head and neck
rows=[(y, 200, 520, 26 if y<300 else 34) for y in range(96,540,20)]
s.group([hc],scallops(rows,w=2.0))
# shadow at the back of the head and neck
shadow=s.clip("M392 90 C470 140 500 220 470 300 C500 330 540 350 560 360 L560 560 L420 560 C440 460 470 380 392 90 Z")
s.group([hc,shadow],hatch(62,5.2,1.6,seed=5))
under=s.clip("M200 330 C260 360 330 380 430 380 L560 380 L560 560 L200 560 Z")
s.group([hc,under],hatch(-30,6,1.4,seed=6))
# bare face patch with fine feather lines
face="M228 162 C246 132 318 128 344 166 C362 198 344 248 304 262 C270 272 240 262 226 248 C220 220 220 188 228 162 Z"
s.path(face,w=4)
fc=s.clip(face)
s.group([fc],curves(lambda t,u:(236+100*t, 176+u*76+10*math.sin(t*6.28+u*3)+ (14*(t-0.5)**2)), 9, 1.3))
# eye
s.add(f'<circle cx="294" cy="194" r="21" fill="{PAPER}" stroke="{INK}" stroke-width="5"/>')
s.add(f'<circle cx="292" cy="196" r="14" fill="{INK}"/><circle cx="287" cy="190" r="4.5" fill="{PAPER}"/><circle cx="298" cy="203" r="1.8" fill="{PAPER}"/>')
s.group([fc],curves(lambda t,u:(294+(28+u*6)*math.cos(math.pi*(0.15+0.9*t)), 194+(28+u*6)*math.sin(math.pi*(0.15+0.9*t))),2,1.4))
# mouth interior
s.path("M222 250 C200 254 178 270 172 296 L238 300 C236 280 232 262 222 250 Z",fill=INK,w=6)
# tongue
s.path("M196 290 C204 278 222 276 232 286 C226 296 210 300 196 290 Z",fill=PAPER,w=3)
# lower bill (open)
lower="M232 300 C214 302 186 298 172 296 C170 314 182 334 206 336 C226 337 238 322 240 304 Z"
s.path(lower,w=7)
lc=s.clip(lower)
s.group([lc],curves(lambda t,u:(176+60*t, 304+u*26+4*math.sin(t*3.1)),6,1.5))
# upper bill: big hook
upper=("M216 172 C178 158 136 184 128 236 C123 270 134 304 152 324 C155 300 162 284 176 272 "
       "C192 260 208 254 226 252 C236 226 232 194 216 172 Z")
s.path(upper,w=8)
uc=s.clip(upper)
s.group([uc],curves(lambda t,u:(212-70*t*(1-0.25*u)+0*u, 176+130*t**1.4*(0.55+0.45*u)+u*10 - 18*math.sin(t*3.1)*(1-u)),10,lambda u:1.2+1.6*u))
# cere / nostril
s.add(f'<path d="M206 176 C214 188 218 204 214 220" fill="none" stroke="{INK}" stroke-width="3.5" stroke-linecap="round"/>')
s.add(f'<ellipse cx="200" cy="186" rx="5" ry="3.5" fill="{INK}"/>')
open(__import__("os").path.join(__import__("os").path.dirname(__file__), "..", "icon.svg"),"w").write(s.out())
