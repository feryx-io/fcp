from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="FirmwareImageResponse")



@_attrs_define
class FirmwareImageResponse:
    """ An uploaded image.

        Attributes:
            id (str):
            vendor (str):
            models (list[str]):
            version (str):
            file_name (str):
            size (int):
            sha256 (str):
            uploaded_at (datetime.datetime):
            notes (None | str | Unset):
     """

    id: str
    vendor: str
    models: list[str]
    version: str
    file_name: str
    size: int
    sha256: str
    uploaded_at: datetime.datetime
    notes: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        vendor = self.vendor

        models = self.models



        version = self.version

        file_name = self.file_name

        size = self.size

        sha256 = self.sha256

        uploaded_at = self.uploaded_at.isoformat()

        notes: None | str | Unset
        if isinstance(self.notes, Unset):
            notes = UNSET
        else:
            notes = self.notes


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "vendor": vendor,
            "models": models,
            "version": version,
            "file_name": file_name,
            "size": size,
            "sha256": sha256,
            "uploaded_at": uploaded_at,
        })
        if notes is not UNSET:
            field_dict["notes"] = notes

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        vendor = d.pop("vendor")

        models = cast(list[str], d.pop("models"))


        version = d.pop("version")

        file_name = d.pop("file_name")

        size = d.pop("size")

        sha256 = d.pop("sha256")

        uploaded_at = datetime.datetime.fromisoformat(d.pop("uploaded_at"))




        def _parse_notes(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        notes = _parse_notes(d.pop("notes", UNSET))


        firmware_image_response = cls(
            id=id,
            vendor=vendor,
            models=models,
            version=version,
            file_name=file_name,
            size=size,
            sha256=sha256,
            uploaded_at=uploaded_at,
            notes=notes,
        )


        firmware_image_response.additional_properties = d
        return firmware_image_response

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
