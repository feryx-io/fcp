from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="EncryptionStatus")



@_attrs_define
class EncryptionStatus:
    """ Snapshot of a backend's encryption state.

        Attributes:
            enabled (bool): Whether secret columns are sealed on write.
            provider (str): `none`, `local`, or `aws-kms`.
            key_ids (list[str]): Every DEK the backend can read with, oldest first.
            sealed_columns (list[str]): `table.column` names that are sealed.
            kek_id (None | str | Unset): Identifier of the key-encryption key wrapping the DEKs.
            active_key_id (None | str | Unset): DEK used for new writes.
     """

    enabled: bool
    provider: str
    key_ids: list[str]
    sealed_columns: list[str]
    kek_id: None | str | Unset = UNSET
    active_key_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        enabled = self.enabled

        provider = self.provider

        key_ids = self.key_ids



        sealed_columns = self.sealed_columns



        kek_id: None | str | Unset
        if isinstance(self.kek_id, Unset):
            kek_id = UNSET
        else:
            kek_id = self.kek_id

        active_key_id: None | str | Unset
        if isinstance(self.active_key_id, Unset):
            active_key_id = UNSET
        else:
            active_key_id = self.active_key_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "enabled": enabled,
            "provider": provider,
            "key_ids": key_ids,
            "sealed_columns": sealed_columns,
        })
        if kek_id is not UNSET:
            field_dict["kek_id"] = kek_id
        if active_key_id is not UNSET:
            field_dict["active_key_id"] = active_key_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        enabled = d.pop("enabled")

        provider = d.pop("provider")

        key_ids = cast(list[str], d.pop("key_ids"))


        sealed_columns = cast(list[str], d.pop("sealed_columns"))


        def _parse_kek_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        kek_id = _parse_kek_id(d.pop("kek_id", UNSET))


        def _parse_active_key_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        active_key_id = _parse_active_key_id(d.pop("active_key_id", UNSET))


        encryption_status = cls(
            enabled=enabled,
            provider=provider,
            key_ids=key_ids,
            sealed_columns=sealed_columns,
            kek_id=kek_id,
            active_key_id=active_key_id,
        )


        encryption_status.additional_properties = d
        return encryption_status

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
