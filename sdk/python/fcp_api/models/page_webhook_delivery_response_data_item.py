from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.page_webhook_delivery_response_data_item_payload import PageWebhookDeliveryResponseDataItemPayload





T = TypeVar("T", bound="PageWebhookDeliveryResponseDataItem")



@_attrs_define
class PageWebhookDeliveryResponseDataItem:
    """ A logged delivery.

        Attributes:
            id (str):
            webhook_id (str):
            event_type (str):
            payload (PageWebhookDeliveryResponseDataItemPayload): The body delivered.
            delivered (bool):
            attempts (int):
            created_at (datetime.datetime):
            status_code (int | None | Unset):
            error (None | str | Unset):
            response_body (None | str | Unset):
            last_attempt_at (datetime.datetime | None | Unset):
            next_retry_at (datetime.datetime | None | Unset): When the dispatcher tries again; none once delivered or given
                up.
     """

    id: str
    webhook_id: str
    event_type: str
    payload: PageWebhookDeliveryResponseDataItemPayload
    delivered: bool
    attempts: int
    created_at: datetime.datetime
    status_code: int | None | Unset = UNSET
    error: None | str | Unset = UNSET
    response_body: None | str | Unset = UNSET
    last_attempt_at: datetime.datetime | None | Unset = UNSET
    next_retry_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.page_webhook_delivery_response_data_item_payload import PageWebhookDeliveryResponseDataItemPayload # noqa: PLC0415
        id = self.id

        webhook_id = self.webhook_id

        event_type = self.event_type

        payload = self.payload.to_dict()

        delivered = self.delivered

        attempts = self.attempts

        created_at = self.created_at.isoformat()

        status_code: int | None | Unset
        if isinstance(self.status_code, Unset):
            status_code = UNSET
        else:
            status_code = self.status_code

        error: None | str | Unset
        if isinstance(self.error, Unset):
            error = UNSET
        else:
            error = self.error

        response_body: None | str | Unset
        if isinstance(self.response_body, Unset):
            response_body = UNSET
        else:
            response_body = self.response_body

        last_attempt_at: None | str | Unset
        if isinstance(self.last_attempt_at, Unset):
            last_attempt_at = UNSET
        elif isinstance(self.last_attempt_at, datetime.datetime):
            last_attempt_at = self.last_attempt_at.isoformat()
        else:
            last_attempt_at = self.last_attempt_at

        next_retry_at: None | str | Unset
        if isinstance(self.next_retry_at, Unset):
            next_retry_at = UNSET
        elif isinstance(self.next_retry_at, datetime.datetime):
            next_retry_at = self.next_retry_at.isoformat()
        else:
            next_retry_at = self.next_retry_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "webhook_id": webhook_id,
            "event_type": event_type,
            "payload": payload,
            "delivered": delivered,
            "attempts": attempts,
            "created_at": created_at,
        })
        if status_code is not UNSET:
            field_dict["status_code"] = status_code
        if error is not UNSET:
            field_dict["error"] = error
        if response_body is not UNSET:
            field_dict["response_body"] = response_body
        if last_attempt_at is not UNSET:
            field_dict["last_attempt_at"] = last_attempt_at
        if next_retry_at is not UNSET:
            field_dict["next_retry_at"] = next_retry_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.page_webhook_delivery_response_data_item_payload import PageWebhookDeliveryResponseDataItemPayload # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        webhook_id = d.pop("webhook_id")

        event_type = d.pop("event_type")

        payload = PageWebhookDeliveryResponseDataItemPayload.from_dict(d.pop("payload"))




        delivered = d.pop("delivered")

        attempts = d.pop("attempts")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        def _parse_status_code(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        status_code = _parse_status_code(d.pop("status_code", UNSET))


        def _parse_error(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        error = _parse_error(d.pop("error", UNSET))


        def _parse_response_body(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        response_body = _parse_response_body(d.pop("response_body", UNSET))


        def _parse_last_attempt_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                last_attempt_at_type_0 = datetime.datetime.fromisoformat(data)



                return last_attempt_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        last_attempt_at = _parse_last_attempt_at(d.pop("last_attempt_at", UNSET))


        def _parse_next_retry_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                next_retry_at_type_0 = datetime.datetime.fromisoformat(data)



                return next_retry_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        next_retry_at = _parse_next_retry_at(d.pop("next_retry_at", UNSET))


        page_webhook_delivery_response_data_item = cls(
            id=id,
            webhook_id=webhook_id,
            event_type=event_type,
            payload=payload,
            delivered=delivered,
            attempts=attempts,
            created_at=created_at,
            status_code=status_code,
            error=error,
            response_body=response_body,
            last_attempt_at=last_attempt_at,
            next_retry_at=next_retry_at,
        )


        page_webhook_delivery_response_data_item.additional_properties = d
        return page_webhook_delivery_response_data_item

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
