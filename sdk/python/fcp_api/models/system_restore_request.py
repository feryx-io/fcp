from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="SystemRestoreRequest")



@_attrs_define
class SystemRestoreRequest:
    """ A full restore asked for: the point's time typed back, and the key file
    when the snapshot holds no key and this system's is another.

        Attributes:
            confirm (str): The point's `time`, exactly.
            key_file (None | str | Unset):
            passphrase (None | str | Unset):
     """

    confirm: str
    key_file: None | str | Unset = UNSET
    passphrase: None | str | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        confirm = self.confirm

        key_file: None | str | Unset
        if isinstance(self.key_file, Unset):
            key_file = UNSET
        else:
            key_file = self.key_file

        passphrase: None | str | Unset
        if isinstance(self.passphrase, Unset):
            passphrase = UNSET
        else:
            passphrase = self.passphrase


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "confirm": confirm,
        })
        if key_file is not UNSET:
            field_dict["key_file"] = key_file
        if passphrase is not UNSET:
            field_dict["passphrase"] = passphrase

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        confirm = d.pop("confirm")

        def _parse_key_file(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        key_file = _parse_key_file(d.pop("key_file", UNSET))


        def _parse_passphrase(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        passphrase = _parse_passphrase(d.pop("passphrase", UNSET))


        system_restore_request = cls(
            confirm=confirm,
            key_file=key_file,
            passphrase=passphrase,
        )

        return system_restore_request

