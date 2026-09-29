from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="QueueStatsResponse")



@_attrs_define
class QueueStatsResponse:
    """ Queue stats response (real-time).

        Attributes:
            queue_name (str):
            calls_waiting (int):
            agents_available (int):
            agents_on_call (int):
            agents_total (int):
            avg_wait_time_secs (float):
            longest_wait_secs (int):
            calls_handled_today (int):
            calls_abandoned_today (int):
            service_level_pct (float):
     """

    queue_name: str
    calls_waiting: int
    agents_available: int
    agents_on_call: int
    agents_total: int
    avg_wait_time_secs: float
    longest_wait_secs: int
    calls_handled_today: int
    calls_abandoned_today: int
    service_level_pct: float
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        queue_name = self.queue_name

        calls_waiting = self.calls_waiting

        agents_available = self.agents_available

        agents_on_call = self.agents_on_call

        agents_total = self.agents_total

        avg_wait_time_secs = self.avg_wait_time_secs

        longest_wait_secs = self.longest_wait_secs

        calls_handled_today = self.calls_handled_today

        calls_abandoned_today = self.calls_abandoned_today

        service_level_pct = self.service_level_pct


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "queue_name": queue_name,
            "calls_waiting": calls_waiting,
            "agents_available": agents_available,
            "agents_on_call": agents_on_call,
            "agents_total": agents_total,
            "avg_wait_time_secs": avg_wait_time_secs,
            "longest_wait_secs": longest_wait_secs,
            "calls_handled_today": calls_handled_today,
            "calls_abandoned_today": calls_abandoned_today,
            "service_level_pct": service_level_pct,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        queue_name = d.pop("queue_name")

        calls_waiting = d.pop("calls_waiting")

        agents_available = d.pop("agents_available")

        agents_on_call = d.pop("agents_on_call")

        agents_total = d.pop("agents_total")

        avg_wait_time_secs = d.pop("avg_wait_time_secs")

        longest_wait_secs = d.pop("longest_wait_secs")

        calls_handled_today = d.pop("calls_handled_today")

        calls_abandoned_today = d.pop("calls_abandoned_today")

        service_level_pct = d.pop("service_level_pct")

        queue_stats_response = cls(
            queue_name=queue_name,
            calls_waiting=calls_waiting,
            agents_available=agents_available,
            agents_on_call=agents_on_call,
            agents_total=agents_total,
            avg_wait_time_secs=avg_wait_time_secs,
            longest_wait_secs=longest_wait_secs,
            calls_handled_today=calls_handled_today,
            calls_abandoned_today=calls_abandoned_today,
            service_level_pct=service_level_pct,
        )


        queue_stats_response.additional_properties = d
        return queue_stats_response

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
