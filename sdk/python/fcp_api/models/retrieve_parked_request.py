from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="RetrieveParkedRequest")



@_attrs_define
class RetrieveParkedRequest:
    """ Where a parked call is sent.

        Attributes:
            target (str): An extension or a user: their phones ring, and answering takes the
                call. Unanswered, their own plan decides (their voicemail).
     """

    target: str





    def to_dict(self) -> dict[str, Any]:
        target = self.target


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "target": target,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        target = d.pop("target")

        retrieve_parked_request = cls(
            target=target,
        )

        return retrieve_parked_request

