from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="MailboxStatsResponse")



@_attrs_define
class MailboxStatsResponse:
    """ Mailbox statistics response.

        Attributes:
            mailbox_id (str):
            total_messages (int):
            new_messages (int):
            heard_messages (int):
            saved_messages (int):
            storage_used (int):
            avg_duration (int):
     """

    mailbox_id: str
    total_messages: int
    new_messages: int
    heard_messages: int
    saved_messages: int
    storage_used: int
    avg_duration: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        mailbox_id = self.mailbox_id

        total_messages = self.total_messages

        new_messages = self.new_messages

        heard_messages = self.heard_messages

        saved_messages = self.saved_messages

        storage_used = self.storage_used

        avg_duration = self.avg_duration


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "mailbox_id": mailbox_id,
            "total_messages": total_messages,
            "new_messages": new_messages,
            "heard_messages": heard_messages,
            "saved_messages": saved_messages,
            "storage_used": storage_used,
            "avg_duration": avg_duration,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        mailbox_id = d.pop("mailbox_id")

        total_messages = d.pop("total_messages")

        new_messages = d.pop("new_messages")

        heard_messages = d.pop("heard_messages")

        saved_messages = d.pop("saved_messages")

        storage_used = d.pop("storage_used")

        avg_duration = d.pop("avg_duration")

        mailbox_stats_response = cls(
            mailbox_id=mailbox_id,
            total_messages=total_messages,
            new_messages=new_messages,
            heard_messages=heard_messages,
            saved_messages=saved_messages,
            storage_used=storage_used,
            avg_duration=avg_duration,
        )


        mailbox_stats_response.additional_properties = d
        return mailbox_stats_response

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
