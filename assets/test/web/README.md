# Original AVIF transform fixtures

These images derive solely from the independently authored 5×3 RGBA16
`alpha.png`, not borrowed conformance artwork. Generated with local avifenc
1.4.2 (aom 3.14.1). The tool is not an application dependency. Quality 100
is lossless for the encoded planes, not the original 16-bit PNG: depth reduction
and RGB→subsampled YUV conversion can lose precision.

Run from the repository root:

```sh
avifenc -l -d 12 -y 444 --cicp 1/13/0 --pasp 2,1 assets/test/web/alpha.png assets/test/web/pasp.avif
avifenc -q 100 --qalpha 100 -d 10 -y 422 --cicp 1/13/1 assets/test/web/alpha.png assets/test/web/crop-source.avif
avifenc -q 100 --qalpha 100 -d 10 -y 422 --cicp 1/13/1 --crop 1,1,3,2 assets/test/web/alpha.png assets/test/web/clap.avif
```

The combined fixture has conflicting EXIF CW orientation plus container CCW
rotation and horizontal mirror to detect double application. Create a little-endian
TIFF payload with one orientation SHORT entry (value 6):

```python
import struct
from pathlib import Path
Path('/tmp/glance-orientation.exif').write_bytes(
    b'II' + struct.pack('<HIH', 42, 8, 1)
    + struct.pack('<HHIHHI', 0x112, 3, 1, 6, 0, 0))
```

```sh
avifenc -l -d 12 -y 444 --cicp 1/13/0 --crop 1,1,3,2 --pasp 2,1 --exif /tmp/glance-orientation.exif --irot 1 --imir 1 assets/test/web/alpha.png assets/test/web/crop-aspect-oriented.avif
avifenc -l -d 8 -y 444 --cicp 1/16/0 assets/test/web/alpha.png assets/test/web/pq.avif
avifenc -l -d 8 -y 444 --cicp 1/18/0 assets/test/web/alpha.png assets/test/web/hlg.avif
avifenc -l -d 8 -y 444 --cicp 1/13/0 assets/test/web/alpha.png assets/test/web/alpha.png assets/test/web/sequence.avif
```

The 4:2:2 crop starts at odd x=1: RGB conversion must precede cropping.
Tests compare all retained samples with the untransformed encoded source,
check U16/alpha retention, aspect dimensions, and exact orientation coordinate
mapping. Existing alpha8/10/12 fixtures remain unchanged.

The external `hato.profile2.12bpc.yuv422.avif` is a read-only local smoke input.
Its redistribution license was not established, so it is not included here.
