from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="WebhookResponse")



@_attrs_define
class WebhookResponse:
    """ A webhook, without its secret.

        Attributes:
            id (str):
            tenant_id (str):
            url (str):
            events (list[str]): Event types delivered: names from `/webhooks/event-types`, a
                category's `call.*`, or `*`. Empty is every event.
            enabled (bool):
            secret_prefix (str): The first characters of the secret, to tell secrets apart.
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            description (None | str | Unset):
            previous_secret_valid_until (datetime.datetime | None | Unset): Until when the secret a rotation replaced also
                signs deliveries.
     """

    id: str
    tenant_id: str
    url: str
    events: list[str]
    enabled: bool
    secret_prefix: str
    created_at: datetime.datetime
    updated_at: datetime.datetime
    description: None | str | Unset = UNSET
    previous_secret_valid_until: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        url = self.url

        events = self.events



        enabled = self.enabled

        secret_prefix = self.secret_prefix

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        previous_secret_valid_until: None | str | Unset
        if isinstance(self.previous_secret_valid_until, Unset):
            previous_secret_valid_until = UNSET
        elif isinstance(self.previous_secret_valid_until, datetime.datetime):
            previous_secret_valid_until = self.previous_secret_valid_until.isoformat()
        else:
            previous_secret_valid_until = self.previous_secret_valid_until


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "url": url,
            "events": events,
            "enabled": enabled,
            "secret_prefix": secret_prefix,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if description is not UNSET:
            field_dict["description"] = description
        if previous_secret_valid_until is not UNSET:
            field_dict["previous_secret_valid_until"] = previous_secret_valid_until

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        url = d.pop("url")

        events = cast(list[str], d.pop("events"))


        enabled = d.pop("enabled")

        secret_prefix = d.pop("secret_prefix")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_previous_secret_valid_until(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                previous_secret_valid_until_type_0 = datetime.datetime.fromisoformat(data)



                return previous_secret_valid_until_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        previous_secret_valid_until = _parse_previous_secret_valid_until(d.pop("previous_secret_valid_until", UNSET))


        webhook_response = cls(
            id=id,
            tenant_id=tenant_id,
            url=url,
            events=events,
            enabled=enabled,
            secret_prefix=secret_prefix,
            created_at=created_at,
            updated_at=updated_at,
            description=description,
            previous_secret_valid_until=previous_secret_valid_until,
        )


        webhook_response.additional_properties = d
        return webhook_response

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
