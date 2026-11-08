from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset






T = TypeVar("T", bound="TenantVideo")



@_attrs_define
class TenantVideo:
    """ A tenant's word on video: calls carry it unless the tenant, or a
    party's class of service, says not.

        Attributes:
            calls (bool | Unset): Calls may carry video; on unless a tenant turns it off.
            max_send_height (int | Unset): The tallest picture a phone's camera sends, in lines (180 to 2160).
            max_send_kbps (int | Unset): The most a phone's camera sends, in kb/s (64 to 8000).
            content_share (bool | Unset): A user may share their screen in a room they don't host.
     """

    calls: bool | Unset = UNSET
    max_send_height: int | Unset = UNSET
    max_send_kbps: int | Unset = UNSET
    content_share: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        calls = self.calls

        max_send_height = self.max_send_height

        max_send_kbps = self.max_send_kbps

        content_share = self.content_share


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if calls is not UNSET:
            field_dict["calls"] = calls
        if max_send_height is not UNSET:
            field_dict["max_send_height"] = max_send_height
        if max_send_kbps is not UNSET:
            field_dict["max_send_kbps"] = max_send_kbps
        if content_share is not UNSET:
            field_dict["content_share"] = content_share

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        calls = d.pop("calls", UNSET)

        max_send_height = d.pop("max_send_height", UNSET)

        max_send_kbps = d.pop("max_send_kbps", UNSET)

        content_share = d.pop("content_share", UNSET)

        tenant_video = cls(
            calls=calls,
            max_send_height=max_send_height,
            max_send_kbps=max_send_kbps,
            content_share=content_share,
        )


        tenant_video.additional_properties = d
        return tenant_video

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
