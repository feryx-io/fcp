from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="KeyExportRequest")



@_attrs_define
class KeyExportRequest:
    """ A passphrase to wrap the key in.

        Attributes:
            passphrase (str): At least 12 characters; kept nowhere, needed to open the file.
     """

    passphrase: str





    def to_dict(self) -> dict[str, Any]:
        passphrase = self.passphrase


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "passphrase": passphrase,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        passphrase = d.pop("passphrase")

        key_export_request = cls(
            passphrase=passphrase,
        )

        return key_export_request

