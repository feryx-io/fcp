from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="DownloadRequest")



@_attrs_define
class DownloadRequest:
    """ What a download needs when the snapshot holds no master key and this
    system's is another: the key file downloaded from this page, and its
    passphrase.

        Attributes:
            key_file (None | str | Unset):
            passphrase (None | str | Unset):
     """

    key_file: None | str | Unset = UNSET
    passphrase: None | str | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
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
        })
        if key_file is not UNSET:
            field_dict["key_file"] = key_file
        if passphrase is not UNSET:
            field_dict["passphrase"] = passphrase

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
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


        download_request = cls(
            key_file=key_file,
            passphrase=passphrase,
        )

        return download_request

