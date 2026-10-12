These two 8×8 solid JPEG fixtures were generated with Pillow 12.3.0:

```python
from PIL import Image
Image.new("L", (8, 8), 96).save("gray_8x8.jpg", quality=100)
Image.new("RGB", (8, 8), (220, 40, 80)).save("rgb_8x8.jpg", quality=100, subsampling=0)
```

The Rust tests consume the committed bytes; Pillow is not a test dependency. The
grayscale image exercises JPEG's one-component input, and the RGB image catches
channel swaps and missing opaque alpha. RGB expectations allow JPEG rounding.
