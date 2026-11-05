"""ASS subtitle files for one scene: captions, lower-third, speed note, sample watermark.

Everything drawn over the video goes through one ASS file per scene, burned in by ffmpeg, so a
watermark in it is on every frame.
"""
from common import ass_time

HEADER = """[Script Info]
ScriptType: v4.00+
PlayResX: 1920
PlayResY: 1080
WrapStyle: 0

[V4+ Styles]
Format: Name,Fontname,Fontsize,PrimaryColour,SecondaryColour,OutlineColour,BackColour,Bold,Italic,Underline,StrikeOut,ScaleX,ScaleY,Spacing,Angle,BorderStyle,Outline,Shadow,Alignment,MarginL,MarginR,MarginV,Encoding
Style: Caption,Inter,44,&H00FFFFFF,&H00FFFFFF,&HB0140D0D,&H00000000,-1,0,0,0,100,100,0,0,3,14,0,2,200,200,70,1
Style: LTitle,Inter,44,&H00FFFFFF,&H00FFFFFF,&HC0140D0D,&H00000000,-1,0,0,0,100,100,0,0,3,16,0,1,0,0,0,1
Style: LSub,Inter,30,&H00C9B8A8,&H00FFFFFF,&HC0140D0D,&H00000000,0,0,0,0,100,100,0,0,3,16,0,1,0,0,0,1
Style: Note,Inter,30,&H0000D2FF,&H00FFFFFF,&HC0140D0D,&H00000000,-1,0,0,0,100,100,0,0,3,12,0,7,60,0,56,1
Style: Mark,Inter,30,&H00FFFFFF,&H00FFFFFF,&H003C32C8,&H00000000,-1,0,0,0,100,100,2,0,3,10,0,9,0,60,50,1
Style: Bar,Inter,20,&H00FFAA58,&H00FFAA58,&H00FFAA58,&H00000000,0,0,0,0,100,100,0,0,1,0,0,7,0,0,0,1

[Events]
Format: Layer,Start,End,Style,Name,MarginL,MarginR,MarginV,Effect,Text
"""


def event(start, end, style, text, layer=0):
    return f"Dialogue: {layer},{ass_time(start)},{ass_time(end)},{style},,0,0,0,,{text}\n"


def captions(caps, offset):
    return [event(offset + c["start"], offset + c["end"], "Caption",
                  wrap(c["text"])) for c in caps]


def wrap(text, width=44):
    """Break a caption into at most two balanced lines."""
    if len(text) <= width:
        return text
    words, best = text.split(), None
    for i in range(1, len(words)):
        a, b = " ".join(words[:i]), " ".join(words[i:])
        score = abs(len(a) - len(b))
        if best is None or score < best[0]:
            best = (score, a, b)
    return f"{best[1]}\\N{best[2]}"


def lower_third(lt, total):
    start, end = lt.get("at", 0.8), min(lt.get("at", 0.8) + lt.get("for", 4.5), total)
    fade = "{\\fad(300,300)}"
    out = [event(start, end, "Bar", fade + "{\\pos(60,720)\\p1}m 0 0 l 8 0 8 112 0 112{\\p0}", 2),
           event(start, end, "LTitle", fade + "{\\pos(84,774)}" + lt["title"], 1)]
    if lt.get("subtitle"):
        out.append(event(start, end, "LSub", fade + "{\\pos(84,824)}" + lt["subtitle"], 1))
    return out


def speed_note(speed, total):
    return [event(0, total, "Note", f"{{\\fad(300,0)}}Sped up {speed:g}x")]


def watermark(text, total):
    return [event(0, total, "Mark", text, 5)]


def scene_ass(total, caps=(), offset=0.0, lt=None, speed=1.0, mark=None):
    ev = captions(caps, offset)
    if lt:
        ev += lower_third(lt, total)
    if speed > 1.05:
        ev += speed_note(round(speed, 1), total)
    if mark:
        ev += watermark(mark, total)
    return HEADER + "".join(ev)
