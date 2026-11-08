from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.softphone_video_send import SoftphoneVideoSend





T = TypeVar("T", bound="SoftphoneVideo")



@_attrs_define
class SoftphoneVideo:
    """ Video on the app (`docs/SOFTPHONE_VIDEO.md` §2.7): whether its calls
    may carry it, and what its camera may send.

        Attributes:
            enabled (bool): The tenant's `video.calls` and the user's class of service; off,
                the app offers no video and shows no video button.
            codecs (list[str]): In order of preference, as SDP names them: what FCP's rooms take
                first. A call relays whatever the two phones agree on.
            max_send (SoftphoneVideoSend): The tenant's cap on what a phone's camera sends.
            content_share (bool): The user may share their screen in a room they don't host.
     """

    enabled: bool
    codecs: list[str]
    max_send: SoftphoneVideoSend
    content_share: bool
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.softphone_video_send import SoftphoneVideoSend # noqa: PLC0415
        enabled = self.enabled

        codecs = self.codecs



        max_send = self.max_send.to_dict()

        content_share = self.content_share


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "enabled": enabled,
            "codecs": codecs,
            "max_send": max_send,
            "content_share": content_share,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.softphone_video_send import SoftphoneVideoSend # noqa: PLC0415
        d = dict(src_dict)
        enabled = d.pop("enabled")

        codecs = cast(list[str], d.pop("codecs"))


        max_send = SoftphoneVideoSend.from_dict(d.pop("max_send"))




        content_share = d.pop("content_share")

        softphone_video = cls(
            enabled=enabled,
            codecs=codecs,
            max_send=max_send,
            content_share=content_share,
        )


        softphone_video.additional_properties = d
        return softphone_video

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
