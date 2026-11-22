from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="RtmpDestinationRequest")



@_attrs_define
class RtmpDestinationRequest:
    """ `PUT …/stream/rtmp` body.

        Attributes:
            url (str): The service's ingest URL, its application in the path:
                `rtmp://a.rtmp.youtube.com/live2`.
            key (None | str | Unset): The stream key. Left out, the key already set is kept.
     """

    url: str
    key: None | str | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        url = self.url

        key: None | str | Unset
        if isinstance(self.key, Unset):
            key = UNSET
        else:
            key = self.key


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "url": url,
        })
        if key is not UNSET:
            field_dict["key"] = key

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        url = d.pop("url")

        def _parse_key(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        key = _parse_key(d.pop("key", UNSET))


        rtmp_destination_request = cls(
            url=url,
            key=key,
        )

        return rtmp_destination_request

