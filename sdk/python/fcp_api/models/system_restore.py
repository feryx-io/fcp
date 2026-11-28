from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.restore_step import RestoreStep





T = TypeVar("T", bound="SystemRestore")



@_attrs_define
class SystemRestore:
    """ A full restore and where it is.

        Attributes:
            id (str):
            state (str): `running`, `succeeded` or `failed`.
            run (str):
            point_time (str):
            started_at (str):
            steps (list[RestoreStep]):
            finished_at (None | str | Unset):
            error (None | str | Unset):
            previous_database (None | str | Unset): The database as it was, renamed and kept, once the restored one is in.
     """

    id: str
    state: str
    run: str
    point_time: str
    started_at: str
    steps: list[RestoreStep]
    finished_at: None | str | Unset = UNSET
    error: None | str | Unset = UNSET
    previous_database: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.restore_step import RestoreStep # noqa: PLC0415
        id = self.id

        state = self.state

        run = self.run

        point_time = self.point_time

        started_at = self.started_at

        steps = []
        for steps_item_data in self.steps:
            steps_item = steps_item_data.to_dict()
            steps.append(steps_item)



        finished_at: None | str | Unset
        if isinstance(self.finished_at, Unset):
            finished_at = UNSET
        else:
            finished_at = self.finished_at

        error: None | str | Unset
        if isinstance(self.error, Unset):
            error = UNSET
        else:
            error = self.error

        previous_database: None | str | Unset
        if isinstance(self.previous_database, Unset):
            previous_database = UNSET
        else:
            previous_database = self.previous_database


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "state": state,
            "run": run,
            "point_time": point_time,
            "started_at": started_at,
            "steps": steps,
        })
        if finished_at is not UNSET:
            field_dict["finished_at"] = finished_at
        if error is not UNSET:
            field_dict["error"] = error
        if previous_database is not UNSET:
            field_dict["previous_database"] = previous_database

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.restore_step import RestoreStep # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        state = d.pop("state")

        run = d.pop("run")

        point_time = d.pop("point_time")

        started_at = d.pop("started_at")

        steps = []
        _steps = d.pop("steps")
        for steps_item_data in (_steps):
            steps_item = RestoreStep.from_dict(steps_item_data)



            steps.append(steps_item)


        def _parse_finished_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        finished_at = _parse_finished_at(d.pop("finished_at", UNSET))


        def _parse_error(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        error = _parse_error(d.pop("error", UNSET))


        def _parse_previous_database(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        previous_database = _parse_previous_database(d.pop("previous_database", UNSET))


        system_restore = cls(
            id=id,
            state=state,
            run=run,
            point_time=point_time,
            started_at=started_at,
            steps=steps,
            finished_at=finished_at,
            error=error,
            previous_database=previous_database,
        )


        system_restore.additional_properties = d
        return system_restore

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
