from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="TenantBackups")



@_attrs_define
class TenantBackups:
    """ What a tenant sees of the platform's backups.

        Attributes:
            times_of_day (list[str]): When the platform backs up each day, `HH:MM` UTC.
            restores_available (bool): Whether lost files can be put back here now.
            last_good_backup_at (None | str | Unset): When the last good backup finished, RFC 3339.
     """

    times_of_day: list[str]
    restores_available: bool
    last_good_backup_at: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        times_of_day = self.times_of_day



        restores_available = self.restores_available

        last_good_backup_at: None | str | Unset
        if isinstance(self.last_good_backup_at, Unset):
            last_good_backup_at = UNSET
        else:
            last_good_backup_at = self.last_good_backup_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "times_of_day": times_of_day,
            "restores_available": restores_available,
        })
        if last_good_backup_at is not UNSET:
            field_dict["last_good_backup_at"] = last_good_backup_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        times_of_day = cast(list[str], d.pop("times_of_day"))


        restores_available = d.pop("restores_available")

        def _parse_last_good_backup_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        last_good_backup_at = _parse_last_good_backup_at(d.pop("last_good_backup_at", UNSET))


        tenant_backups = cls(
            times_of_day=times_of_day,
            restores_available=restores_available,
            last_good_backup_at=last_good_backup_at,
        )


        tenant_backups.additional_properties = d
        return tenant_backups

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
