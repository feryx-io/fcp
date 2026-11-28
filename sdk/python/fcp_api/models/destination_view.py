from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.retention_view import RetentionView
  from ..models.run_view import RunView
  from ..models.schedule_view import ScheduleView
  from ..models.target_view_type_0 import TargetViewType0
  from ..models.target_view_type_1 import TargetViewType1
  from ..models.target_view_type_2 import TargetViewType2
  from ..models.target_view_type_3 import TargetViewType3
  from ..models.target_view_type_4 import TargetViewType4





T = TypeVar("T", bound="DestinationView")



@_attrs_define
class DestinationView:
    """ A backup destination.

        Attributes:
            id (str): `bkd-<uuid>`.
            name (str):
            target (TargetViewType0 | TargetViewType1 | TargetViewType2 | TargetViewType3 | TargetViewType4): A
                destination's target, its secrets never shown.
            has_password (bool):
            schedule (ScheduleView):
            keep (RetentionView):
            include_key (bool):
            enabled (bool):
            repository (str): The repository's address as restic names it, for restoring by hand.
            overdue (bool): Scheduled, and no good backup for a day and two hours.
            created_at (str):
            updated_at (str):
            last_run (None | RunView | Unset):
            last_check (None | RunView | Unset):
            last_success_at (None | str | Unset): When the last good backup finished.
            password (None | str | Unset): Shown once, when a destination is made without a password and one
                is generated: keep it, it opens every snapshot.
     """

    id: str
    name: str
    target: TargetViewType0 | TargetViewType1 | TargetViewType2 | TargetViewType3 | TargetViewType4
    has_password: bool
    schedule: ScheduleView
    keep: RetentionView
    include_key: bool
    enabled: bool
    repository: str
    overdue: bool
    created_at: str
    updated_at: str
    last_run: None | RunView | Unset = UNSET
    last_check: None | RunView | Unset = UNSET
    last_success_at: None | str | Unset = UNSET
    password: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.retention_view import RetentionView # noqa: PLC0415
        from ..models.run_view import RunView # noqa: PLC0415
        from ..models.schedule_view import ScheduleView # noqa: PLC0415
        from ..models.target_view_type_0 import TargetViewType0 # noqa: PLC0415
        from ..models.target_view_type_1 import TargetViewType1 # noqa: PLC0415
        from ..models.target_view_type_2 import TargetViewType2 # noqa: PLC0415
        from ..models.target_view_type_3 import TargetViewType3 # noqa: PLC0415
        from ..models.target_view_type_4 import TargetViewType4 # noqa: PLC0415
        id = self.id

        name = self.name

        target: dict[str, Any]
        if isinstance(self.target, TargetViewType0):
            target = self.target.to_dict()
        elif isinstance(self.target, TargetViewType1):
            target = self.target.to_dict()
        elif isinstance(self.target, TargetViewType2):
            target = self.target.to_dict()
        elif isinstance(self.target, TargetViewType3):
            target = self.target.to_dict()
        else:
            target = self.target.to_dict()


        has_password = self.has_password

        schedule = self.schedule.to_dict()

        keep = self.keep.to_dict()

        include_key = self.include_key

        enabled = self.enabled

        repository = self.repository

        overdue = self.overdue

        created_at = self.created_at

        updated_at = self.updated_at

        last_run: dict[str, Any] | None | Unset
        if isinstance(self.last_run, Unset):
            last_run = UNSET
        elif isinstance(self.last_run, RunView):
            last_run = self.last_run.to_dict()
        else:
            last_run = self.last_run

        last_check: dict[str, Any] | None | Unset
        if isinstance(self.last_check, Unset):
            last_check = UNSET
        elif isinstance(self.last_check, RunView):
            last_check = self.last_check.to_dict()
        else:
            last_check = self.last_check

        last_success_at: None | str | Unset
        if isinstance(self.last_success_at, Unset):
            last_success_at = UNSET
        else:
            last_success_at = self.last_success_at

        password: None | str | Unset
        if isinstance(self.password, Unset):
            password = UNSET
        else:
            password = self.password


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "name": name,
            "target": target,
            "has_password": has_password,
            "schedule": schedule,
            "keep": keep,
            "include_key": include_key,
            "enabled": enabled,
            "repository": repository,
            "overdue": overdue,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if last_run is not UNSET:
            field_dict["last_run"] = last_run
        if last_check is not UNSET:
            field_dict["last_check"] = last_check
        if last_success_at is not UNSET:
            field_dict["last_success_at"] = last_success_at
        if password is not UNSET:
            field_dict["password"] = password

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.retention_view import RetentionView # noqa: PLC0415
        from ..models.run_view import RunView # noqa: PLC0415
        from ..models.schedule_view import ScheduleView # noqa: PLC0415
        from ..models.target_view_type_0 import TargetViewType0 # noqa: PLC0415
        from ..models.target_view_type_1 import TargetViewType1 # noqa: PLC0415
        from ..models.target_view_type_2 import TargetViewType2 # noqa: PLC0415
        from ..models.target_view_type_3 import TargetViewType3 # noqa: PLC0415
        from ..models.target_view_type_4 import TargetViewType4 # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        name = d.pop("name")

        def _parse_target(data: object) -> TargetViewType0 | TargetViewType1 | TargetViewType2 | TargetViewType3 | TargetViewType4:
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                componentsschemas_target_view_type_0 = TargetViewType0.from_dict(data)



                return componentsschemas_target_view_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                componentsschemas_target_view_type_1 = TargetViewType1.from_dict(data)



                return componentsschemas_target_view_type_1
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                componentsschemas_target_view_type_2 = TargetViewType2.from_dict(data)



                return componentsschemas_target_view_type_2
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                componentsschemas_target_view_type_3 = TargetViewType3.from_dict(data)



                return componentsschemas_target_view_type_3
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            if not isinstance(data, dict):
                raise TypeError()
            componentsschemas_target_view_type_4 = TargetViewType4.from_dict(data)



            return componentsschemas_target_view_type_4

        target = _parse_target(d.pop("target"))


        has_password = d.pop("has_password")

        schedule = ScheduleView.from_dict(d.pop("schedule"))




        keep = RetentionView.from_dict(d.pop("keep"))




        include_key = d.pop("include_key")

        enabled = d.pop("enabled")

        repository = d.pop("repository")

        overdue = d.pop("overdue")

        created_at = d.pop("created_at")

        updated_at = d.pop("updated_at")

        def _parse_last_run(data: object) -> None | RunView | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                last_run_type_0 = RunView.from_dict(data)



                return last_run_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | RunView | Unset, data)

        last_run = _parse_last_run(d.pop("last_run", UNSET))


        def _parse_last_check(data: object) -> None | RunView | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                last_check_type_0 = RunView.from_dict(data)



                return last_check_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | RunView | Unset, data)

        last_check = _parse_last_check(d.pop("last_check", UNSET))


        def _parse_last_success_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        last_success_at = _parse_last_success_at(d.pop("last_success_at", UNSET))


        def _parse_password(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        password = _parse_password(d.pop("password", UNSET))


        destination_view = cls(
            id=id,
            name=name,
            target=target,
            has_password=has_password,
            schedule=schedule,
            keep=keep,
            include_key=include_key,
            enabled=enabled,
            repository=repository,
            overdue=overdue,
            created_at=created_at,
            updated_at=updated_at,
            last_run=last_run,
            last_check=last_check,
            last_success_at=last_success_at,
            password=password,
        )


        destination_view.additional_properties = d
        return destination_view

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
