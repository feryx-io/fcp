from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="BackupStatus")



@_attrs_define
class BackupStatus:
    """ A destination as a site's link reports it up (`docs/SCHEDULED_BACKUPS.md`
    B5): no secrets, no target beyond its kind.

        Attributes:
            destination_id (str): `bkd-<uuid>`, at the site.
            name (str):
            kind (str): `sftp`, `s3`, `directory`, `rest` or `rclone`.
            enabled (bool):
            scheduled (bool):
            overdue (bool):
            last_success_at (None | str | Unset): RFC 3339.
            last_run_state (None | str | Unset): The last backup run's state and when it was asked for.
            last_run_at (None | str | Unset):
     """

    destination_id: str
    name: str
    kind: str
    enabled: bool
    scheduled: bool
    overdue: bool
    last_success_at: None | str | Unset = UNSET
    last_run_state: None | str | Unset = UNSET
    last_run_at: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        destination_id = self.destination_id

        name = self.name

        kind = self.kind

        enabled = self.enabled

        scheduled = self.scheduled

        overdue = self.overdue

        last_success_at: None | str | Unset
        if isinstance(self.last_success_at, Unset):
            last_success_at = UNSET
        else:
            last_success_at = self.last_success_at

        last_run_state: None | str | Unset
        if isinstance(self.last_run_state, Unset):
            last_run_state = UNSET
        else:
            last_run_state = self.last_run_state

        last_run_at: None | str | Unset
        if isinstance(self.last_run_at, Unset):
            last_run_at = UNSET
        else:
            last_run_at = self.last_run_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "destination_id": destination_id,
            "name": name,
            "kind": kind,
            "enabled": enabled,
            "scheduled": scheduled,
            "overdue": overdue,
        })
        if last_success_at is not UNSET:
            field_dict["last_success_at"] = last_success_at
        if last_run_state is not UNSET:
            field_dict["last_run_state"] = last_run_state
        if last_run_at is not UNSET:
            field_dict["last_run_at"] = last_run_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        destination_id = d.pop("destination_id")

        name = d.pop("name")

        kind = d.pop("kind")

        enabled = d.pop("enabled")

        scheduled = d.pop("scheduled")

        overdue = d.pop("overdue")

        def _parse_last_success_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        last_success_at = _parse_last_success_at(d.pop("last_success_at", UNSET))


        def _parse_last_run_state(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        last_run_state = _parse_last_run_state(d.pop("last_run_state", UNSET))


        def _parse_last_run_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        last_run_at = _parse_last_run_at(d.pop("last_run_at", UNSET))


        backup_status = cls(
            destination_id=destination_id,
            name=name,
            kind=kind,
            enabled=enabled,
            scheduled=scheduled,
            overdue=overdue,
            last_success_at=last_success_at,
            last_run_state=last_run_state,
            last_run_at=last_run_at,
        )


        backup_status.additional_properties = d
        return backup_status

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
