// Owned opaque adapter compiled against the installed libavif header.
// Rust never duplicates a libavif struct or enum layout.
#include <avif/avif.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>

typedef struct {
    avifDecoder *decoder;
    avifRGBImage rgb;
} glance_avif;

void glance_avif_free(glance_avif *h) {
    if (!h) return;
    avifRGBImageFreePixels(&h->rgb);
    avifDecoderDestroy(h->decoder);
    free(h);
}

int glance_avif_decode(const uint8_t *data, size_t length, glance_avif **out, char *error, size_t error_size) {
    *out = NULL;
    int status = 1;
    avifResult result = AVIF_RESULT_OK;
    glance_avif *h = calloc(1, sizeof(*h));
    if (!h) { snprintf(error, error_size, "allocation failed"); return 3; }
    h->decoder = avifDecoderCreate();
    if (!h->decoder) { free(h); snprintf(error, error_size, "decoder allocation failed"); return 3; }
    avifDecoder *d = h->decoder;
    d->codecChoice = AVIF_CODEC_CHOICE_DAV1D;
    d->maxThreads = 2;
    d->imageDimensionLimit = 16384;
    d->imageSizeLimit = (512U * 1024U * 1024U) / 8U;
    d->imageCountLimit = 2;
    d->ignoreXMP = AVIF_TRUE;
    if (!avifCodecName(AVIF_CODEC_CHOICE_DAV1D, AVIF_CODEC_FLAG_CAN_DECODE)) {
        snprintf(error, error_size, "libavif was not built with the dav1d decoder"); status = 2; goto fail;
    }
    result = avifDecoderSetIOMemory(d, data, length);
    if (result != AVIF_RESULT_OK) goto codec_error;
    result = avifDecoderParse(d);
    if (result != AVIF_RESULT_OK) goto codec_error;
    if (d->imageCount != 1 || d->imageSequenceTrackPresent) {
        snprintf(error, error_size, "sequences are not supported yet"); status = 2; goto fail;
    }
    avifImage *im = d->image;
    if (im->transformFlags & (AVIF_TRANSFORM_CLAP | AVIF_TRANSFORM_PASP)) {
        snprintf(error, error_size, "clean aperture / non-square pixel transforms are not supported"); status = 2; goto fail;
    }
    if (im->depth != 8 && im->depth != 10 && im->depth != 12) {
        snprintf(error, error_size, "only 8/10/12-bit stills are supported"); status = 2; goto fail;
    }
    if (!im->width || !im->height || im->width > 16384 || im->height > 16384 ||
        (uint64_t)im->width * im->height * 8 > 512ULL * 1024 * 1024) {
        snprintf(error, error_size, "dimensions exceed decoded budget"); status = 3; goto fail;
    }
    result = avifDecoderNextImage(d);
    if (result != AVIF_RESULT_OK) goto codec_error;
    avifRGBImageSetDefaults(&h->rgb, im);
    h->rgb.depth = 16;
    h->rgb.format = AVIF_RGB_FORMAT_RGBA;
    h->rgb.avoidLibYUV = AVIF_TRUE;
    h->rgb.alphaPremultiplied = AVIF_FALSE;
    result = avifRGBImageAllocatePixels(&h->rgb);
    if (result != AVIF_RESULT_OK) goto codec_error;
    result = avifImageYUVToRGB(im, &h->rgb);
    if (result != AVIF_RESULT_OK) goto codec_error;
    *out = h;
    return 0;
codec_error:
    snprintf(error, error_size, "%s: %s", avifResultToString(result), d->diag.error);
fail:
    glance_avif_free(h);
    return status;
}

uint32_t glance_avif_width(const glance_avif *h) { return h->rgb.width; }
uint32_t glance_avif_height(const glance_avif *h) { return h->rgb.height; }
uint32_t glance_avif_stride(const glance_avif *h) { return h->rgb.rowBytes; }
const uint8_t *glance_avif_pixels(const glance_avif *h) { return h->rgb.pixels; }
const uint8_t *glance_avif_icc(const glance_avif *h, size_t *length) {
    *length = h->decoder->image->icc.size; return h->decoder->image->icc.data;
}
const uint8_t *glance_avif_exif(const glance_avif *h, size_t *length) {
    *length = h->decoder->image->exif.size; return h->decoder->image->exif.data;
}
uint32_t glance_avif_transformed(const glance_avif *h) { return h->decoder->image->transformFlags & (AVIF_TRANSFORM_IROT | AVIF_TRANSFORM_IMIR); }
uint32_t glance_avif_rotation(const glance_avif *h) {
    const avifImage *im = h->decoder->image;
    return (im->transformFlags & AVIF_TRANSFORM_IROT) ? im->irot.angle : 0;
}
uint32_t glance_avif_mirror(const glance_avif *h) {
    const avifImage *im = h->decoder->image;
    return (im->transformFlags & AVIF_TRANSFORM_IMIR) ? im->imir.axis : 2;
}
uint32_t glance_avif_primaries(const glance_avif *h) { return h->decoder->image->colorPrimaries; }
uint32_t glance_avif_transfer(const glance_avif *h) { return h->decoder->image->transferCharacteristics; }
