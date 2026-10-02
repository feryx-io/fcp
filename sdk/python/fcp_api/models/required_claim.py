from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="RequiredClaim")



@_attrs_define
class RequiredClaim:
    """ A claim a token must carry, with this value.

        Attributes:
            claim (str):
            value (str):
     """

    claim: str
    value: str





    def to_dict(self) -> dict[str, Any]:
        claim = self.claim

        value = self.value


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "claim": claim,
            "value": value,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        claim = d.pop("claim")

        value = d.pop("value")

        required_claim = cls(
            claim=claim,
            value=value,
        )

        return required_claim

