# Run by gpurender through `blender --background --python convert_image.py -- <image> <out.bmp>`
# Converts any image Blender can read (png, jpg, tga, tiff, exr ...) into a 32-bit BMP.
import sys

import bpy

src, dst = sys.argv[sys.argv.index("--") + 1:][:2]
img = bpy.data.images.load(src)
scene = bpy.context.scene
scene.render.image_settings.file_format = "BMP"
try:
    scene.render.image_settings.color_mode = "RGBA"
    img.save_render(dst, scene=scene)
except Exception:
    scene.render.image_settings.color_mode = "RGB"
    img.save_render(dst, scene=scene)
print("GPURENDER: converted")
