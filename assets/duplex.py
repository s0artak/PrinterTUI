"""Manual duplex tutorial animation for a rear-tray printer (e.g. HP Smart Tank 5100).

Rebuild assets/duplex.mp4:
    blender -b -P assets/duplex.py -- /path/to/frames/
    ffmpeg -y -framerate 24 -i /path/to/frames/%04d.png -c:v libx264 -pix_fmt yuv420p -crf 28 assets/duplex.mp4
"""

import sys
import bpy
from mathutils import Matrix, Vector

OUT = sys.argv[sys.argv.index("--") + 1] if "--" in sys.argv else "//frames/"
FPS, END = 24, 330

for o in list(bpy.data.objects):
    bpy.data.objects.remove(o)
scene = bpy.context.scene
col = scene.collection


def obj(name, data, color, parent=None, loc=(0, 0, 0), rot=(0, 0, 0)):
    o = bpy.data.objects.new(name, data)
    o.color = (*color, 1)
    o.parent = parent
    o.location = loc
    o.rotation_euler = rot
    col.objects.link(o)
    return o


def box(name, size, loc, grey):
    x, y, z = (s / 2 for s in size)
    v = [(-x, -y, -z), (x, -y, -z), (x, y, -z), (-x, y, -z), (-x, -y, z), (x, -y, z), (x, y, z), (-x, y, z)]
    f = [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
    m = bpy.data.meshes.new(name)
    m.from_pydata(v, [], f)
    return obj(name, m, (grey,) * 3, loc=loc)


def text(name, body, size, color, parent=None, loc=(0, 0, 0), rot=(0, 0, 0)):
    c = bpy.data.curves.new(name, "FONT")
    c.body, c.size = body, size
    c.align_x, c.align_y = "CENTER", "CENTER"
    return obj(name, c, color, parent, loc, rot)


def show(o, frame, visible):
    o.hide_render = o.hide_viewport = not visible
    o.keyframe_insert("hide_render", frame=frame)
    o.keyframe_insert("hide_viewport", frame=frame)


# Printer: body, rear input tray, front output tray, output slot.
box("body", (4, 3, 1.4), (0, 0, 0.7), 0.35)
box("input_tray", (2.2, 0.06, 2.6), (0, 1.53, 2.3), 0.22)
box("output_tray", (2.2, 2.4, 0.04), (0, -2.7, 0.25), 0.22)
box("slot", (2.0, 0.02, 0.12), (0, -1.51, 0.35), 0.05)

# Sheets: local +Z is the front face, local +Y the top edge of the page.
W, H = 1.6, 2.26
sheets = []
for n, (front, back) in enumerate([("1", "2"), ("3", "4")]):
    m = bpy.data.meshes.new(f"sheet{n}")
    m.from_pydata([(-W / 2, -H / 2, 0), (W / 2, -H / 2, 0), (W / 2, H / 2, 0), (-W / 2, H / 2, 0)], [], [(0, 1, 2, 3)])
    s = obj(f"sheet{n}", m, (0.95,) * 3)
    s.rotation_mode = "QUATERNION"
    faces = []
    for label, z, flip in ((front, 0.006, 0), (back, -0.006, 3.14159)):
        faces.append([
            text(f"p{label}", label, 1.3, (0.02,) * 3, s, (0, -0.1, z), (0, flip, 0)),
            text(f"t{label}", "TOP", 0.2, (0.02,) * 3, s, (0, 0.9, z), (0, flip, 0)),
        ])
    sheets.append((s, faces))


def pose(y, z):
    y, z = Vector(y), Vector(z)
    return Matrix((y.cross(z), y, z)).transposed().to_quaternion()


BLANK_IN = pose((0, 0, -1), (0, -1, 0))    # rear tray, blank side facing you, top edge down
OUT_FRONT = pose((0, -1, 0), (0, 0, 1))    # output tray, front page face up
FLIPPED_IN = pose((0, 0, -1), (0, 1, 0))   # rear tray, printed side facing the back, top edge down
OUT_BACK = pose((0, -1, 0), (0, 0, -1))    # output tray, back page face up
REAR_Z = 1.4 + H / 2 + 0.08


def key(s, frame, loc, q):
    prev = s.rotation_quaternion
    s.location, s.rotation_quaternion = loc, q if prev.dot(q) >= 0 else -q
    s.keyframe_insert("location", frame=frame)
    s.keyframe_insert("rotation_quaternion", frame=frame)


def feed(s, t, rear_y, q_in, q_out, out_z):
    """Sheet sinks into the rear slot, then slides out of the front slot."""
    key(s, t, (0, rear_y, REAR_Z), q_in)
    key(s, t + 10, (0, rear_y, REAR_Z - 1.3), q_in)
    key(s, t + 11, (0, -0.35, 0.35), q_out)  # hidden inside the body
    key(s, t + 26, (0, -2.7, out_z), q_out)


(s1, f1), (s3, f3) = sheets
for s, faces in sheets:
    for o in faces[0] + faces[1]:
        show(o, 1, False)

# 1. Front pages print.
key(s1, 1, (0, 1.36, REAR_Z), BLANK_IN)
key(s3, 1, (0, 1.40, REAR_Z), BLANK_IN)
feed(s1, 12, 1.36, BLANK_IN, OUT_FRONT, 0.29)
feed(s3, 44, 1.40, BLANK_IN, OUT_FRONT, 0.31)
for o in f1[0]:
    show(o, 23, True)
for o in f3[0]:
    show(o, 55, True)

# 2-3. Take the stack as it is and flip it into the rear tray.
for s, dz in ((s1, 0.0), (s3, 0.02)):
    key(s, 100, (0, -2.7, 0.29 + dz), OUT_FRONT)
    key(s, 118, (0, -2.7, 1.6 + dz), OUT_FRONT)
    mid = OUT_FRONT.slerp(FLIPPED_IN, 0.5)
    key(s, 160, Vector((0, -0.6, 3.0)) + mid @ Vector((0, 0, dz)), mid)
    key(s, 200, Vector((0, 1.38, REAR_Z)) + FLIPPED_IN @ Vector((0, 0, dz)), FLIPPED_IN)

# 4-5. Back pages print.
feed(s1, 240, 1.38, FLIPPED_IN, OUT_BACK, 0.29)
feed(s3, 272, 1.40, FLIPPED_IN, OUT_BACK, 0.31)
for o in f1[1]:
    show(o, 251, True)
for o in f3[1]:
    show(o, 283, True)

# Camera and captions.
cam = obj("camera", bpy.data.cameras.new("camera"), (1, 1, 1), loc=(9.5, -11, 9))
target = Vector((0, -0.6, 1.2))
cam.rotation_euler = (target - cam.location).to_track_quat("-Z", "Y").to_euler()
scene.camera = cam
cam.data.lens, cam.data.shift_y = 40, -0.05
bar = box("caption_bar", (4, 0.34, 0.01), (0, -0.84, -4.02), 0.0)
bar.parent = cam

CAPTIONS = [
    (1, "1. PrinterTUI prints the FRONT pages (1, 3)"),
    (90, "2. Wait until it finishes, then take the whole stack\nwithout changing the order"),
    (130, "3. Flip it: printed side facing the BACK of the printer,\nTOP of the page going in first (pointing down)"),
    (210, "4. Put it in the input tray and press Enter in PrinterTUI"),
    (250, "5. The BACK pages print (2, 4). Done!"),
]
for i, (start, body) in enumerate(CAPTIONS):
    c = text(f"caption{i}", body, 0.085, (1, 1, 1), cam, (0, -0.84, -4))
    stop = CAPTIONS[i + 1][0] if i + 1 < len(CAPTIONS) else END + 1
    show(c, 1, start == 1)
    if start > 1:
        show(c, start, True)
    show(c, stop, False)

# Render: flat monochrome workbench.
scene.render.engine = "BLENDER_WORKBENCH"
sh = scene.display.shading
sh.light, sh.color_type = "FLAT", "OBJECT"
sh.show_object_outline, sh.object_outline_color = True, (0, 0, 0)
scene.display.render_aa = "8"
scene.world = scene.world or bpy.data.worlds.new("world")
scene.world.color = (0.06,) * 3
scene.view_settings.view_transform = "Standard"
scene.render.resolution_x, scene.render.resolution_y = 1280, 720
scene.render.fps, scene.frame_start, scene.frame_end = FPS, 1, END
scene.render.image_settings.file_format = "PNG"
scene.render.filepath = OUT
