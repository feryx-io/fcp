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
  from ..models.schedule_view import ScheduleView
  from ..models.target_request_type_0 import TargetRequestType0
  from ..models.target_request_type_1 import TargetRequestType1





T = TypeVar("T", bound="DestinationRequest")



@_attrs_define
class DestinationRequest:
    """ A destination to make or change.

        Attributes:
            name (str):
            target (TargetRequestType0 | TargetRequestType1): A destination's target to keep; a secret left out on a change
                keeps the
                one kept.
            password (None | str | Unset): The repository password; write-only. Left out when a destination is
                made, one is generated and shown once.
            schedule (None | ScheduleView | Unset):
            keep (None | RetentionView | Unset):
            include_key (bool | Unset):
            enabled (bool | Unset):
     """

    name: str
    target: TargetRequestType0 | TargetRequestType1
    password: None | str | Unset = UNSET
    schedule: None | ScheduleView | Unset = UNSET
    keep: None | RetentionView | Unset = UNSET
    include_key: bool | Unset = UNSET
    enabled: bool | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        from ..models.retention_view import RetentionView # noqa: PLC0415
        from ..models.schedule_view import ScheduleView # noqa: PLC0415
        from ..models.target_request_type_0 import TargetRequestType0 # noqa: PLC0415
        from ..models.target_request_type_1 import TargetRequestType1 # noqa: PLC0415
        name = self.name

        target: dict[str, Any]
        if isinstance(self.target, TargetRequestType0):
            target = self.target.to_dict()
        else:
            target = self.target.to_dict()


        password: None | str | Unset
        if isinstance(self.password, Unset):
            password = UNSET
        else:
            password = self.password

        schedule: dict[str, Any] | None | Unset
        if isinstance(self.schedule, Unset):
            schedule = UNSET
        elif isinstance(self.schedule, ScheduleView):
            schedule = self.schedule.to_dict()
        else:
            schedule = self.schedule

        keep: dict[str, Any] | None | Unset
        if isinstance(self.keep, Unset):
            keep = UNSET
        elif isinstance(self.keep, RetentionView):
            keep = self.keep.to_dict()
        else:
            keep = self.keep

        include_key = self.include_key

        enabled = self.enabled


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "name": name,
            "target": target,
        })
        if password is not UNSET:
            field_dict["password"] = password
        if schedule is not UNSET:
            field_dict["schedule"] = schedule
        if keep is not UNSET:
            field_dict["keep"] = keep
        if include_key is not UNSET:
            field_dict["include_key"] = include_key
        if enabled is not UNSET:
            field_dict["enabled"] = enabled

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.retention_view import RetentionView # noqa: PLC0415
        from ..models.schedule_view import ScheduleView # noqa: PLC0415
        from ..models.target_request_type_0 import TargetRequestType0 # noqa: PLC0415
        from ..models.target_request_type_1 import TargetRequestType1 # noqa: PLC0415
        d = dict(src_dict)
        name = d.pop("name")

        def _parse_target(data: object) -> TargetRequestType0 | TargetRequestType1:
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                componentsschemas_target_request_type_0 = TargetRequestType0.from_dict(data)



                return componentsschemas_target_request_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            if not isinstance(data, dict):
                raise TypeError()
            componentsschemas_target_request_type_1 = TargetRequestType1.from_dict(data)



            return componentsschemas_target_request_type_1

        target = _parse_target(d.pop("target"))


        def _parse_password(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        password = _parse_password(d.pop("password", UNSET))


        def _parse_schedule(data: object) -> None | ScheduleView | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                schedule_type_0 = ScheduleView.from_dict(data)



                return schedule_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | ScheduleView | Unset, data)

        schedule = _parse_schedule(d.pop("schedule", UNSET))


        def _parse_keep(data: object) -> None | RetentionView | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                keep_type_0 = RetentionView.from_dict(data)



                return keep_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | RetentionView | Unset, data)

        keep = _parse_keep(d.pop("keep", UNSET))


        include_key = d.pop("include_key", UNSET)

        enabled = d.pop("enabled", UNSET)

        destination_request = cls(
            name=name,
            target=target,
            password=password,
            schedule=schedule,
            keep=keep,
            include_key=include_key,
            enabled=enabled,
        )

        return destination_request

