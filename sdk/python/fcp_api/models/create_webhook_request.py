from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="CreateWebhookRequest")



@_attrs_define
class CreateWebhookRequest:
    """ What a webhook is made with.

        Attributes:
            url (str):
            events (list[str] | Unset):
            enabled (bool | Unset):
            description (None | str | Unset):
     """

    url: str
    events: list[str] | Unset = UNSET
    enabled: bool | Unset = UNSET
    description: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        url = self.url

        events: list[str] | Unset = UNSET
        if not isinstance(self.events, Unset):
            events = self.events



        enabled = self.enabled

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "url": url,
        })
        if events is not UNSET:
            field_dict["events"] = events
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if description is not UNSET:
            field_dict["description"] = description

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        url = d.pop("url")

        events = cast(list[str], d.pop("events", UNSET))


        enabled = d.pop("enabled", UNSET)

        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        create_webhook_request = cls(
            url=url,
            events=events,
            enabled=enabled,
            description=description,
        )


        create_webhook_request.additional_properties = d
        return create_webhook_request

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
