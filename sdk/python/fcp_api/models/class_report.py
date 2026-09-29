from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.missing_file import MissingFile





T = TypeVar("T", bound="ClassReport")



@_attrs_define
class ClassReport:
    """ What one class of data gave up to an export.

        Attributes:
            items (int): Items that are the subject's.
            files (int | Unset): Media files put in the bundle.
            missing_files (list[MissingFile] | Unset): Media files that are the subject's but could not be read from this
                host, with why.
            held (int | Unset): Items under a legal hold (exported like the rest).
     """

    items: int
    files: int | Unset = UNSET
    missing_files: list[MissingFile] | Unset = UNSET
    held: int | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.missing_file import MissingFile # noqa: PLC0415
        items = self.items

        files = self.files

        missing_files: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.missing_files, Unset):
            missing_files = []
            for missing_files_item_data in self.missing_files:
                missing_files_item = missing_files_item_data.to_dict()
                missing_files.append(missing_files_item)



        held = self.held


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "items": items,
        })
        if files is not UNSET:
            field_dict["files"] = files
        if missing_files is not UNSET:
            field_dict["missing_files"] = missing_files
        if held is not UNSET:
            field_dict["held"] = held

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.missing_file import MissingFile # noqa: PLC0415
        d = dict(src_dict)
        items = d.pop("items")

        files = d.pop("files", UNSET)

        _missing_files = d.pop("missing_files", UNSET)
        missing_files: list[MissingFile] | Unset = UNSET
        if _missing_files is not UNSET:
            missing_files = []
            for missing_files_item_data in _missing_files:
                missing_files_item = MissingFile.from_dict(missing_files_item_data)



                missing_files.append(missing_files_item)


        held = d.pop("held", UNSET)

        class_report = cls(
            items=items,
            files=files,
            missing_files=missing_files,
            held=held,
        )


        class_report.additional_properties = d
        return class_report

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
