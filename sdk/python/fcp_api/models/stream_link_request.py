from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="StreamLinkRequest")



@_attrs_define
class StreamLinkRequest:
    """ `POST …/stream/links` body.

        Attributes:
            ttl_secs (int | None | Unset): The link's lifetime in seconds: a day by default, a week at most.
     """

    ttl_secs: int | None | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        ttl_secs: int | None | Unset
        if isinstance(self.ttl_secs, Unset):
            ttl_secs = UNSET
        else:
            ttl_secs = self.ttl_secs


        field_dict: dict[str, Any] = {}

        field_dict.update({
        })
        if ttl_secs is not UNSET:
            field_dict["ttl_secs"] = ttl_secs

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_ttl_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        ttl_secs = _parse_ttl_secs(d.pop("ttl_secs", UNSET))


        stream_link_request = cls(
            ttl_secs=ttl_secs,
        )

        return stream_link_request

