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
  from ..models.emergency_location_response import EmergencyLocationResponse





T = TypeVar("T", bound="PageEmergencyCallResponseDataItem")



@_attrs_define
class PageEmergencyCallResponseDataItem:
    """ 
        Attributes:
            id (int):
            tenant_id (str):
            call_id (str):
            caller_uri (str):
            dialed_number (str):
            kind (str):
            outcome (str):
            placed_at (datetime.datetime):
            notified (bool):
            sip_call_id (None | str | Unset):
            caller_username (None | str | Unset):
            callback_number (None | str | Unset):
            location (EmergencyLocationResponse | None | Unset):
            trunk_name (None | str | Unset):
            trunk_id (None | str | Unset): Which trunk carried it; a name is unique only within its tenant.
            sip_status (int | None | Unset): The final SIP status, once the call has ended.
            answered_at (datetime.datetime | None | Unset):
            ended_at (datetime.datetime | None | Unset):
            duration_secs (int | None | Unset): Seconds from answer to end, for an answered call.
            delivery (None | str | Unset): How the location travelled: `elin`, `pidf_lo`, `elin+pidf_lo`, or none.
            location_structured (bool | None | Unset): Whether the location's street went out in civic elements.
            geolocation_error (None | str | Unset): The `Geolocation-Error` a trunk answered the location with.
     """

    id: int
    tenant_id: str
    call_id: str
    caller_uri: str
    dialed_number: str
    kind: str
    outcome: str
    placed_at: datetime.datetime
    notified: bool
    sip_call_id: None | str | Unset = UNSET
    caller_username: None | str | Unset = UNSET
    callback_number: None | str | Unset = UNSET
    location: EmergencyLocationResponse | None | Unset = UNSET
    trunk_name: None | str | Unset = UNSET
    trunk_id: None | str | Unset = UNSET
    sip_status: int | None | Unset = UNSET
    answered_at: datetime.datetime | None | Unset = UNSET
    ended_at: datetime.datetime | None | Unset = UNSET
    duration_secs: int | None | Unset = UNSET
    delivery: None | str | Unset = UNSET
    location_structured: bool | None | Unset = UNSET
    geolocation_error: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.emergency_location_response import EmergencyLocationResponse # noqa: PLC0415
        id = self.id

        tenant_id = self.tenant_id

        call_id = self.call_id

        caller_uri = self.caller_uri

        dialed_number = self.dialed_number

        kind = self.kind

        outcome = self.outcome

        placed_at = self.placed_at.isoformat()

        notified = self.notified

        sip_call_id: None | str | Unset
        if isinstance(self.sip_call_id, Unset):
            sip_call_id = UNSET
        else:
            sip_call_id = self.sip_call_id

        caller_username: None | str | Unset
        if isinstance(self.caller_username, Unset):
            caller_username = UNSET
        else:
            caller_username = self.caller_username

        callback_number: None | str | Unset
        if isinstance(self.callback_number, Unset):
            callback_number = UNSET
        else:
            callback_number = self.callback_number

        location: dict[str, Any] | None | Unset
        if isinstance(self.location, Unset):
            location = UNSET
        elif isinstance(self.location, EmergencyLocationResponse):
            location = self.location.to_dict()
        else:
            location = self.location

        trunk_name: None | str | Unset
        if isinstance(self.trunk_name, Unset):
            trunk_name = UNSET
        else:
            trunk_name = self.trunk_name

        trunk_id: None | str | Unset
        if isinstance(self.trunk_id, Unset):
            trunk_id = UNSET
        else:
            trunk_id = self.trunk_id

        sip_status: int | None | Unset
        if isinstance(self.sip_status, Unset):
            sip_status = UNSET
        else:
            sip_status = self.sip_status

        answered_at: None | str | Unset
        if isinstance(self.answered_at, Unset):
            answered_at = UNSET
        elif isinstance(self.answered_at, datetime.datetime):
            answered_at = self.answered_at.isoformat()
        else:
            answered_at = self.answered_at

        ended_at: None | str | Unset
        if isinstance(self.ended_at, Unset):
            ended_at = UNSET
        elif isinstance(self.ended_at, datetime.datetime):
            ended_at = self.ended_at.isoformat()
        else:
            ended_at = self.ended_at

        duration_secs: int | None | Unset
        if isinstance(self.duration_secs, Unset):
            duration_secs = UNSET
        else:
            duration_secs = self.duration_secs

        delivery: None | str | Unset
        if isinstance(self.delivery, Unset):
            delivery = UNSET
        else:
            delivery = self.delivery

        location_structured: bool | None | Unset
        if isinstance(self.location_structured, Unset):
            location_structured = UNSET
        else:
            location_structured = self.location_structured

        geolocation_error: None | str | Unset
        if isinstance(self.geolocation_error, Unset):
            geolocation_error = UNSET
        else:
            geolocation_error = self.geolocation_error


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "call_id": call_id,
            "caller_uri": caller_uri,
            "dialed_number": dialed_number,
            "kind": kind,
            "outcome": outcome,
            "placed_at": placed_at,
            "notified": notified,
        })
        if sip_call_id is not UNSET:
            field_dict["sip_call_id"] = sip_call_id
        if caller_username is not UNSET:
            field_dict["caller_username"] = caller_username
        if callback_number is not UNSET:
            field_dict["callback_number"] = callback_number
        if location is not UNSET:
            field_dict["location"] = location
        if trunk_name is not UNSET:
            field_dict["trunk_name"] = trunk_name
        if trunk_id is not UNSET:
            field_dict["trunk_id"] = trunk_id
        if sip_status is not UNSET:
            field_dict["sip_status"] = sip_status
        if answered_at is not UNSET:
            field_dict["answered_at"] = answered_at
        if ended_at is not UNSET:
            field_dict["ended_at"] = ended_at
        if duration_secs is not UNSET:
            field_dict["duration_secs"] = duration_secs
        if delivery is not UNSET:
            field_dict["delivery"] = delivery
        if location_structured is not UNSET:
            field_dict["location_structured"] = location_structured
        if geolocation_error is not UNSET:
            field_dict["geolocation_error"] = geolocation_error

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.emergency_location_response import EmergencyLocationResponse # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        call_id = d.pop("call_id")

        caller_uri = d.pop("caller_uri")

        dialed_number = d.pop("dialed_number")

        kind = d.pop("kind")

        outcome = d.pop("outcome")

        placed_at = datetime.datetime.fromisoformat(d.pop("placed_at"))




        notified = d.pop("notified")

        def _parse_sip_call_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        sip_call_id = _parse_sip_call_id(d.pop("sip_call_id", UNSET))


        def _parse_caller_username(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        caller_username = _parse_caller_username(d.pop("caller_username", UNSET))


        def _parse_callback_number(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        callback_number = _parse_callback_number(d.pop("callback_number", UNSET))


        def _parse_location(data: object) -> EmergencyLocationResponse | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                location_type_0 = EmergencyLocationResponse.from_dict(data)



                return location_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(EmergencyLocationResponse | None | Unset, data)

        location = _parse_location(d.pop("location", UNSET))


        def _parse_trunk_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        trunk_name = _parse_trunk_name(d.pop("trunk_name", UNSET))


        def _parse_trunk_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        trunk_id = _parse_trunk_id(d.pop("trunk_id", UNSET))


        def _parse_sip_status(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        sip_status = _parse_sip_status(d.pop("sip_status", UNSET))


        def _parse_answered_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                answered_at_type_0 = datetime.datetime.fromisoformat(data)



                return answered_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        answered_at = _parse_answered_at(d.pop("answered_at", UNSET))


        def _parse_ended_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                ended_at_type_0 = datetime.datetime.fromisoformat(data)



                return ended_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        ended_at = _parse_ended_at(d.pop("ended_at", UNSET))


        def _parse_duration_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        duration_secs = _parse_duration_secs(d.pop("duration_secs", UNSET))


        def _parse_delivery(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        delivery = _parse_delivery(d.pop("delivery", UNSET))


        def _parse_location_structured(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        location_structured = _parse_location_structured(d.pop("location_structured", UNSET))


        def _parse_geolocation_error(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        geolocation_error = _parse_geolocation_error(d.pop("geolocation_error", UNSET))


        page_emergency_call_response_data_item = cls(
            id=id,
            tenant_id=tenant_id,
            call_id=call_id,
            caller_uri=caller_uri,
            dialed_number=dialed_number,
            kind=kind,
            outcome=outcome,
            placed_at=placed_at,
            notified=notified,
            sip_call_id=sip_call_id,
            caller_username=caller_username,
            callback_number=callback_number,
            location=location,
            trunk_name=trunk_name,
            trunk_id=trunk_id,
            sip_status=sip_status,
            answered_at=answered_at,
            ended_at=ended_at,
            duration_secs=duration_secs,
            delivery=delivery,
            location_structured=location_structured,
            geolocation_error=geolocation_error,
        )


        page_emergency_call_response_data_item.additional_properties = d
        return page_emergency_call_response_data_item

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
