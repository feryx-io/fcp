from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="NotificationResponse")



@_attrs_define
class NotificationResponse:
    """ One attempt to tell one contact about one emergency call (§8.3).

        Attributes:
            id (int):
            call_id (str):
            kind (str):
            target (str):
            attempt (int):
            status (str): `delivered`, `retrying` or `failed`.
            test (bool): Whether this told someone about a test call.
            created_at (datetime.datetime):
            call_record_id (int | None | Unset):
            name (None | str | Unset):
            error (None | str | Unset):
            delivered_at (datetime.datetime | None | Unset):
            next_retry_at (datetime.datetime | None | Unset):
     """

    id: int
    call_id: str
    kind: str
    target: str
    attempt: int
    status: str
    test: bool
    created_at: datetime.datetime
    call_record_id: int | None | Unset = UNSET
    name: None | str | Unset = UNSET
    error: None | str | Unset = UNSET
    delivered_at: datetime.datetime | None | Unset = UNSET
    next_retry_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        call_id = self.call_id

        kind = self.kind

        target = self.target

        attempt = self.attempt

        status = self.status

        test = self.test

        created_at = self.created_at.isoformat()

        call_record_id: int | None | Unset
        if isinstance(self.call_record_id, Unset):
            call_record_id = UNSET
        else:
            call_record_id = self.call_record_id

        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        error: None | str | Unset
        if isinstance(self.error, Unset):
            error = UNSET
        else:
            error = self.error

        delivered_at: None | str | Unset
        if isinstance(self.delivered_at, Unset):
            delivered_at = UNSET
        elif isinstance(self.delivered_at, datetime.datetime):
            delivered_at = self.delivered_at.isoformat()
        else:
            delivered_at = self.delivered_at

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
            "call_id": call_id,
            "kind": kind,
            "target": target,
            "attempt": attempt,
            "status": status,
            "test": test,
            "created_at": created_at,
        })
        if call_record_id is not UNSET:
            field_dict["call_record_id"] = call_record_id
        if name is not UNSET:
            field_dict["name"] = name
        if error is not UNSET:
            field_dict["error"] = error
        if delivered_at is not UNSET:
            field_dict["delivered_at"] = delivered_at
        if next_retry_at is not UNSET:
            field_dict["next_retry_at"] = next_retry_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        call_id = d.pop("call_id")

        kind = d.pop("kind")

        target = d.pop("target")

        attempt = d.pop("attempt")

        status = d.pop("status")

        test = d.pop("test")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        def _parse_call_record_id(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        call_record_id = _parse_call_record_id(d.pop("call_record_id", UNSET))


        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        def _parse_error(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        error = _parse_error(d.pop("error", UNSET))


        def _parse_delivered_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                delivered_at_type_0 = datetime.datetime.fromisoformat(data)



                return delivered_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        delivered_at = _parse_delivered_at(d.pop("delivered_at", UNSET))


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


        notification_response = cls(
            id=id,
            call_id=call_id,
            kind=kind,
            target=target,
            attempt=attempt,
            status=status,
            test=test,
            created_at=created_at,
            call_record_id=call_record_id,
            name=name,
            error=error,
            delivered_at=delivered_at,
            next_retry_at=next_retry_at,
        )


        notification_response.additional_properties = d
        return notification_response

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
