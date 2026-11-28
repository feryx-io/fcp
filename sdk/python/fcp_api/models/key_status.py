from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="KeyStatus")



@_attrs_define
class KeyStatus:
    """ Whether the master key can be downloaded here, and when it last was.

        Attributes:
            provider (str): `local`, `aws-kms` or `none`.
            exportable (bool): A local key can be downloaded; a KMS key never leaves the KMS.
            kek_id (None | str | Unset):
            last_exported_at (None | str | Unset): When this KEK was last downloaded; until then the page asks for it.
            last_exported_by (None | str | Unset):
     """

    provider: str
    exportable: bool
    kek_id: None | str | Unset = UNSET
    last_exported_at: None | str | Unset = UNSET
    last_exported_by: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        provider = self.provider

        exportable = self.exportable

        kek_id: None | str | Unset
        if isinstance(self.kek_id, Unset):
            kek_id = UNSET
        else:
            kek_id = self.kek_id

        last_exported_at: None | str | Unset
        if isinstance(self.last_exported_at, Unset):
            last_exported_at = UNSET
        else:
            last_exported_at = self.last_exported_at

        last_exported_by: None | str | Unset
        if isinstance(self.last_exported_by, Unset):
            last_exported_by = UNSET
        else:
            last_exported_by = self.last_exported_by


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "provider": provider,
            "exportable": exportable,
        })
        if kek_id is not UNSET:
            field_dict["kek_id"] = kek_id
        if last_exported_at is not UNSET:
            field_dict["last_exported_at"] = last_exported_at
        if last_exported_by is not UNSET:
            field_dict["last_exported_by"] = last_exported_by

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        provider = d.pop("provider")

        exportable = d.pop("exportable")

        def _parse_kek_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        kek_id = _parse_kek_id(d.pop("kek_id", UNSET))


        def _parse_last_exported_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        last_exported_at = _parse_last_exported_at(d.pop("last_exported_at", UNSET))


        def _parse_last_exported_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        last_exported_by = _parse_last_exported_by(d.pop("last_exported_by", UNSET))


        key_status = cls(
            provider=provider,
            exportable=exportable,
            kek_id=kek_id,
            last_exported_at=last_exported_at,
            last_exported_by=last_exported_by,
        )


        key_status.additional_properties = d
        return key_status

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
