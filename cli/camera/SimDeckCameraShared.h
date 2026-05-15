#pragma once

#include <stdint.h>

#define SIMDECK_CAMERA_MAGIC 0x4d434453u
#define SIMDECK_CAMERA_VERSION 1u
#define SIMDECK_CAMERA_HEADER_SIZE 4096u
#define SIMDECK_CAMERA_SOURCE_PLACEHOLDER 1u
#define SIMDECK_CAMERA_SOURCE_IMAGE 2u
#define SIMDECK_CAMERA_SOURCE_VIDEO 3u
#define SIMDECK_CAMERA_SOURCE_WEBCAM 4u
#define SIMDECK_CAMERA_MIRROR_AUTO 0u
#define SIMDECK_CAMERA_MIRROR_OFF 1u
#define SIMDECK_CAMERA_MIRROR_ON 2u

typedef struct SimDeckCameraHeader {
    uint32_t magic;
    uint32_t version;
    uint32_t headerSize;
    uint32_t width;
    uint32_t height;
    uint32_t bytesPerRow;
    uint32_t pixelFormat;
    uint32_t sourceKind;
    volatile uint64_t sequence;
    uint64_t timestampNs;
    uint32_t mirrorMode;
    uint32_t reserved;
    char sourceLabel[240];
} SimDeckCameraHeader;

static inline uint64_t SimDeckCameraBufferSize(uint32_t width, uint32_t height) {
    return (uint64_t)SIMDECK_CAMERA_HEADER_SIZE + ((uint64_t)width * 4u * (uint64_t)height);
}
