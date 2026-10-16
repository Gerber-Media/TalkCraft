#!/usr/bin/env python3
"""Builds the sample episode the tutorial pictures are made from (macOS only: uses `say`).

    python3 site/tutorial/make_audio.py OUT_DIR

Writes per language (de, en) into OUT_DIR/<lang>/:
  <speaker>.wav         one per speaker, named like its track, silent while the other talks
  Intro.wav             a short intro chime
  cover.png             the episode cover (1400 px square)
  script.json           every line with speaker, start/end seconds and words (rough timings)
The episode has what the tutorials need: filler words, a slip that is said again, a long pause.
"""
import json
import math
import os
import struct
import subprocess
import sys
import tempfile
import wave

RATE = 48_000
INTRO = 3.5  # the jingle plays first
GAP = 0.45   # between two lines
LONG_PAUSE = 3.2

EPISODES = {
    "de": {
        "voices": {"host": "Anna", "guest": "Eddy (Deutsch (Deutschland))"},
        "names": {"host": "Moderation", "guest": "Gast 1"},
        "fillers": {"ähm", "äh"},
        "lines": [
            ("host", "Herzlich willkommen zu Folge zwölf. Heute geht es ums Podcasten zu Hause."),
            ("guest", "Hallo, schön, dass ich da sein darf."),
            ("host", "Ähm, wie hast du eigentlich mit dem Podcasten angefangen?"),
            ("guest", "Mit einem alten Mikrofon und, ähm, sehr viel Geduld. Der Raum war viel zu hallig."),
            ("pause", ""),
            ("host", "Was ist dein wichtigster Trick für, äh, Moment, das sage ich nochmal."),
            ("host", "Was ist dein wichtigster Tipp für Einsteiger?"),
            ("guest", "Nah ans Mikrofon gehen und weiche Möbel in den Raum stellen. Das hilft mehr als jedes teure Gerät."),
            ("host", "Das nehmen wir mit. Danke dir!"),
        ],
        "slip": 5,  # index of the line that is said again
    },
    "en": {
        "voices": {"host": "Samantha", "guest": "Daniel"},
        "names": {"host": "Host", "guest": "Guest 1"},
        "fillers": {"um", "uh"},
        "lines": [
            ("host", "Welcome to episode twelve. Today we talk about podcasting from home."),
            ("guest", "Hi, great to be here."),
            ("host", "Um, how did you actually get started with podcasting?"),
            ("guest", "With an old microphone and, um, a lot of patience. The room was far too echoey."),
            ("pause", ""),
            ("host", "What is your most important trick for, uh, hold on, let me say that again."),
            ("host", "What is your most important tip for beginners?"),
            ("guest", "Get close to the microphone and put soft furniture in the room. That helps more than any expensive gear."),
            ("host", "Great advice. Thank you!"),
        ],
        "slip": 5,
    },
}


def say(voice, text, path):
    subprocess.run(["say", "-v", voice, "--data-format=LEI16@48000", "-o", path, text], check=True)
    with wave.open(path, "rb") as w:
        frames = w.readframes(w.getnframes())
    samples = list(struct.unpack(f"<{len(frames) // 2}h", frames))
    # Trim the silence `say` leaves around the speech.
    loud = [i for i, s in enumerate(samples) if abs(s) > 300]
    return samples[loud[0]:loud[-1] + 1] if loud else samples


def write(path, samples):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(struct.pack(f"<{len(samples)}h", *samples))


def chime(seconds=3.0):
    """Three soft rising tones with a fade: a recognisable intro jingle."""
    out = []
    notes = [(523.25, 0.0), (659.25, 0.35), (783.99, 0.7)]
    for i in range(int(seconds * RATE)):
        t = i / RATE
        v = 0.0
        for f, at in notes:
            if t >= at:
                d = t - at
                v += math.sin(2 * math.pi * f * t) * math.exp(-1.6 * d) * min(1.0, d * 40)
        fade = min(1.0, (seconds - t) * 2)
        out.append(int(max(-1.0, min(1.0, v * 0.28 * fade)) * 32767))
    return out


def cover(path):
    """The episode cover: TalkCraft's parrot, square at 1400 px as podcast apps want it."""
    from PIL import Image
    icon = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "assets", "app-icon", "talkcraft-1024.png")
    Image.open(icon).convert("RGB").resize((1400, 1400), Image.LANCZOS).save(path)


def words_of(text, start, end, fillers):
    """Splits a line into words and spreads its time over them by length (good enough for the window)."""
    parts = text.split()
    weights = [len(p) + 2 for p in parts]
    total = sum(weights)
    out, t = [], start
    for p, wgt in zip(parts, weights):
        d = (end - start) * wgt / total
        bare = p.strip(".,!?").lower()
        out.append({"text": p, "start": round(t, 3), "end": round(t + d, 3), "filler": bare in fillers})
        t += d
    return out


def build(lang, spec, out):
    os.makedirs(out, exist_ok=True)
    tracks = {"host": [], "guest": []}
    script, t = [], INTRO
    with tempfile.TemporaryDirectory() as tmp:
        for n, (who, text) in enumerate(spec["lines"]):
            if who == "pause":
                t += LONG_PAUSE - GAP
                continue
            speech = say(spec["voices"][who], text, os.path.join(tmp, f"{n}.wav"))
            begin = int(t * RATE)
            for track in tracks.values():
                track.extend([0] * (begin - len(track)))
            tracks[who].extend(speech)
            end = t + len(speech) / RATE
            script.append({
                "speaker": who,
                "name": spec["names"][who],
                "text": text,
                "start": round(t, 3),
                "end": round(end, 3),
                "slip": n == spec["slip"],
                "words": words_of(text, t, end, spec["fillers"]),
            })
            t = end + GAP
    length = max(len(v) for v in tracks.values())
    for who, track in tracks.items():
        track.extend([0] * (length - len(track)))
        # Named like the track: the clip takes the file's name.
        write(os.path.join(out, f"{spec['names'][who]}.wav"), track)
    write(os.path.join(out, "Intro.wav"), chime())
    cover(os.path.join(out, "cover.png"))
    with open(os.path.join(out, "script.json"), "w", encoding="utf-8") as f:
        json.dump({"lang": lang, "names": spec["names"], "length": round(length / RATE, 3), "lines": script}, f, ensure_ascii=False, indent=1)
    print(f"{lang}: {length / RATE:.1f} s, {len(script)} lines")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    for lang, spec in EPISODES.items():
        build(lang, spec, os.path.join(sys.argv[1], lang))
