from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast






T = TypeVar("T", bound="SoftphoneMedia")



@_attrs_define
class SoftphoneMedia:
    """ The media the app offers.

        Attributes:
            codecs (list[str]): In order of preference, as SDP names them.
            srtp (str): `optional` over TLS (SDES-SRTP offered, plain RTP taken), else `off`.
            dtmf (str): `rfc2833`.
     """

    codecs: list[str]
    srtp: str
    dtmf: str
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        codecs = self.codecs



        srtp = self.srtp

        dtmf = self.dtmf


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "codecs": codecs,
            "srtp": srtp,
            "dtmf": dtmf,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        codecs = cast(list[str], d.pop("codecs"))


        srtp = d.pop("srtp")

        dtmf = d.pop("dtmf")

        softphone_media = cls(
            codecs=codecs,
            srtp=srtp,
            dtmf=dtmf,
        )


        softphone_media.additional_properties = d
        return softphone_media

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
