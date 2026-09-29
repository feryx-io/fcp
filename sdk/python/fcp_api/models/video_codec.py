from enum import StrEnum

class VideoCodec(StrEnum):
    AV1 = "AV1"
    H264 = "H264"
    H265 = "H265"
    VP8 = "VP8"
    VP9 = "VP9"

    def __str__(self) -> str:
        return str(self.value)
