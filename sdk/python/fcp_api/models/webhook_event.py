from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.webhook_event_event_type import WebhookEventEventType
from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.webhook_event_data import WebhookEventData





T = TypeVar("T", bound="WebhookEvent")



@_attrs_define
class WebhookEvent:
    """ The body of every delivery (the spec's `webhooks` section).

        Attributes:
            event_id (str): Unique per delivery: a receiver may drop one it has seen.
            event_type (WebhookEventEventType): A name from `/api/v1/webhooks/event-types`, or `webhook.test`
            timestamp (datetime.datetime): When the event happened, RFC 3339.
            data (WebhookEventData): The event itself.
            tenant_id (None | str | Unset):
     """

    event_id: str
    event_type: WebhookEventEventType
    timestamp: datetime.datetime
    data: WebhookEventData
    tenant_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.webhook_event_data import WebhookEventData # noqa: PLC0415
        event_id = self.event_id

        event_type = self.event_type.value

        timestamp = self.timestamp.isoformat()

        data = self.data.to_dict()

        tenant_id: None | str | Unset
        if isinstance(self.tenant_id, Unset):
            tenant_id = UNSET
        else:
            tenant_id = self.tenant_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "event_id": event_id,
            "event_type": event_type,
            "timestamp": timestamp,
            "data": data,
        })
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.webhook_event_data import WebhookEventData # noqa: PLC0415
        d = dict(src_dict)
        event_id = d.pop("event_id")

        event_type = WebhookEventEventType(d.pop("event_type"))




        timestamp = datetime.datetime.fromisoformat(d.pop("timestamp"))




        data = WebhookEventData.from_dict(d.pop("data"))




        def _parse_tenant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tenant_id = _parse_tenant_id(d.pop("tenant_id", UNSET))


        webhook_event = cls(
            event_id=event_id,
            event_type=event_type,
            timestamp=timestamp,
            data=data,
            tenant_id=tenant_id,
        )


        webhook_event.additional_properties = d
        return webhook_event

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
