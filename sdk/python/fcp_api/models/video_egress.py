from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="VideoEgress")



@_attrs_define
class VideoEgress:
    """ What a conference node may still send, in kilobits a second: every
    composite and forwarded stream leaving it shares its link.

        Attributes:
            left_kbps (int):
            total_kbps (int):
     """

    left_kbps: int
    total_kbps: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        left_kbps = self.left_kbps

        total_kbps = self.total_kbps


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "left_kbps": left_kbps,
            "total_kbps": total_kbps,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        left_kbps = d.pop("left_kbps")

        total_kbps = d.pop("total_kbps")

        video_egress = cls(
            left_kbps=left_kbps,
            total_kbps=total_kbps,
        )


        video_egress.additional_properties = d
        return video_egress

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
