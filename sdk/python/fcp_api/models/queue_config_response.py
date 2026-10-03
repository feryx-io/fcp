from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="QueueConfigResponse")



@_attrs_define
class QueueConfigResponse:
    """ Queue config response.

        Attributes:
            id (str):
            tenant_id (str):
            name (str):
            extension (str):
            strategy (str):
            max_wait_time_secs (int):
            max_queue_depth (int):
            announce_position (bool):
            announce_interval_secs (int):
            retry_on_no_answer (bool):
            agents (list[str]):
            priority (int):
            required_skills (list[str]):
            record_calls (bool):
            callback_enabled (bool):
            timezone (str):
            enabled (bool):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            overflow_destination (None | str | Unset):
            moh_class (None | str | Unset):
            schedule_id (None | str | Unset): The queue's hours and where a caller goes outside them (§3.5).
            closed_target (None | str | Unset):
            agent_group_id (None | str | Unset): A group whose members may log in to the queue besides `agents`.
     """

    id: str
    tenant_id: str
    name: str
    extension: str
    strategy: str
    max_wait_time_secs: int
    max_queue_depth: int
    announce_position: bool
    announce_interval_secs: int
    retry_on_no_answer: bool
    agents: list[str]
    priority: int
    required_skills: list[str]
    record_calls: bool
    callback_enabled: bool
    timezone: str
    enabled: bool
    created_at: datetime.datetime
    updated_at: datetime.datetime
    overflow_destination: None | str | Unset = UNSET
    moh_class: None | str | Unset = UNSET
    schedule_id: None | str | Unset = UNSET
    closed_target: None | str | Unset = UNSET
    agent_group_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        name = self.name

        extension = self.extension

        strategy = self.strategy

        max_wait_time_secs = self.max_wait_time_secs

        max_queue_depth = self.max_queue_depth

        announce_position = self.announce_position

        announce_interval_secs = self.announce_interval_secs

        retry_on_no_answer = self.retry_on_no_answer

        agents = self.agents



        priority = self.priority

        required_skills = self.required_skills



        record_calls = self.record_calls

        callback_enabled = self.callback_enabled

        timezone = self.timezone

        enabled = self.enabled

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        overflow_destination: None | str | Unset
        if isinstance(self.overflow_destination, Unset):
            overflow_destination = UNSET
        else:
            overflow_destination = self.overflow_destination

        moh_class: None | str | Unset
        if isinstance(self.moh_class, Unset):
            moh_class = UNSET
        else:
            moh_class = self.moh_class

        schedule_id: None | str | Unset
        if isinstance(self.schedule_id, Unset):
            schedule_id = UNSET
        else:
            schedule_id = self.schedule_id

        closed_target: None | str | Unset
        if isinstance(self.closed_target, Unset):
            closed_target = UNSET
        else:
            closed_target = self.closed_target

        agent_group_id: None | str | Unset
        if isinstance(self.agent_group_id, Unset):
            agent_group_id = UNSET
        else:
            agent_group_id = self.agent_group_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "name": name,
            "extension": extension,
            "strategy": strategy,
            "max_wait_time_secs": max_wait_time_secs,
            "max_queue_depth": max_queue_depth,
            "announce_position": announce_position,
            "announce_interval_secs": announce_interval_secs,
            "retry_on_no_answer": retry_on_no_answer,
            "agents": agents,
            "priority": priority,
            "required_skills": required_skills,
            "record_calls": record_calls,
            "callback_enabled": callback_enabled,
            "timezone": timezone,
            "enabled": enabled,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if overflow_destination is not UNSET:
            field_dict["overflow_destination"] = overflow_destination
        if moh_class is not UNSET:
            field_dict["moh_class"] = moh_class
        if schedule_id is not UNSET:
            field_dict["schedule_id"] = schedule_id
        if closed_target is not UNSET:
            field_dict["closed_target"] = closed_target
        if agent_group_id is not UNSET:
            field_dict["agent_group_id"] = agent_group_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        name = d.pop("name")

        extension = d.pop("extension")

        strategy = d.pop("strategy")

        max_wait_time_secs = d.pop("max_wait_time_secs")

        max_queue_depth = d.pop("max_queue_depth")

        announce_position = d.pop("announce_position")

        announce_interval_secs = d.pop("announce_interval_secs")

        retry_on_no_answer = d.pop("retry_on_no_answer")

        agents = cast(list[str], d.pop("agents"))


        priority = d.pop("priority")

        required_skills = cast(list[str], d.pop("required_skills"))


        record_calls = d.pop("record_calls")

        callback_enabled = d.pop("callback_enabled")

        timezone = d.pop("timezone")

        enabled = d.pop("enabled")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_overflow_destination(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        overflow_destination = _parse_overflow_destination(d.pop("overflow_destination", UNSET))


        def _parse_moh_class(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        moh_class = _parse_moh_class(d.pop("moh_class", UNSET))


        def _parse_schedule_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        schedule_id = _parse_schedule_id(d.pop("schedule_id", UNSET))


        def _parse_closed_target(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        closed_target = _parse_closed_target(d.pop("closed_target", UNSET))


        def _parse_agent_group_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        agent_group_id = _parse_agent_group_id(d.pop("agent_group_id", UNSET))


        queue_config_response = cls(
            id=id,
            tenant_id=tenant_id,
            name=name,
            extension=extension,
            strategy=strategy,
            max_wait_time_secs=max_wait_time_secs,
            max_queue_depth=max_queue_depth,
            announce_position=announce_position,
            announce_interval_secs=announce_interval_secs,
            retry_on_no_answer=retry_on_no_answer,
            agents=agents,
            priority=priority,
            required_skills=required_skills,
            record_calls=record_calls,
            callback_enabled=callback_enabled,
            timezone=timezone,
            enabled=enabled,
            created_at=created_at,
            updated_at=updated_at,
            overflow_destination=overflow_destination,
            moh_class=moh_class,
            schedule_id=schedule_id,
            closed_target=closed_target,
            agent_group_id=agent_group_id,
        )


        queue_config_response.additional_properties = d
        return queue_config_response

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
