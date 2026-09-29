from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast






T = TypeVar("T", bound="AgentResponse")



@_attrs_define
class AgentResponse:
    """ Agent info response.

        Attributes:
            id (str):
            extension (str):
            state (str):
            queues (list[str]):
            calls_handled (int):
            priority (int):
     """

    id: str
    extension: str
    state: str
    queues: list[str]
    calls_handled: int
    priority: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        extension = self.extension

        state = self.state

        queues = self.queues



        calls_handled = self.calls_handled

        priority = self.priority


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "extension": extension,
            "state": state,
            "queues": queues,
            "calls_handled": calls_handled,
            "priority": priority,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        extension = d.pop("extension")

        state = d.pop("state")

        queues = cast(list[str], d.pop("queues"))


        calls_handled = d.pop("calls_handled")

        priority = d.pop("priority")

        agent_response = cls(
            id=id,
            extension=extension,
            state=state,
            queues=queues,
            calls_handled=calls_handled,
            priority=priority,
        )


        agent_response.additional_properties = d
        return agent_response

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
