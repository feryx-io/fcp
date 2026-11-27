from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="PlatformSpeechCapRequest")



@_attrs_define
class PlatformSpeechCapRequest:
    """ 
        Attributes:
            monthly_characters (int):
     """

    monthly_characters: int





    def to_dict(self) -> dict[str, Any]:
        monthly_characters = self.monthly_characters


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "monthly_characters": monthly_characters,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        monthly_characters = d.pop("monthly_characters")

        platform_speech_cap_request = cls(
            monthly_characters=monthly_characters,
        )

        return platform_speech_cap_request

