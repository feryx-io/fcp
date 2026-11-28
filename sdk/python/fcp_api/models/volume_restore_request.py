from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="VolumeRestoreRequest")



@_attrs_define
class VolumeRestoreRequest:
    """ A volume's files to put back.

        Attributes:
            volume (str): The compose volume (`voicemail-data`).
            paths (list[str] | Unset): Paths inside it; none is every file it is missing.
     """

    volume: str
    paths: list[str] | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        volume = self.volume

        paths: list[str] | Unset = UNSET
        if not isinstance(self.paths, Unset):
            paths = self.paths




        field_dict: dict[str, Any] = {}

        field_dict.update({
            "volume": volume,
        })
        if paths is not UNSET:
            field_dict["paths"] = paths

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        volume = d.pop("volume")

        paths = cast(list[str], d.pop("paths", UNSET))


        volume_restore_request = cls(
            volume=volume,
            paths=paths,
        )

        return volume_restore_request

