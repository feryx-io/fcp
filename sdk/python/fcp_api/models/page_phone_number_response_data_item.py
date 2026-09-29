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
  from ..models.e911_address_response import E911AddressResponse





T = TypeVar("T", bound="PagePhoneNumberResponseDataItem")



@_attrs_define
class PagePhoneNumberResponseDataItem:
    """ Phone number response.

        Attributes:
            id (str):
            e164 (str):
            country_code (str):
            npa (str):
            nxx (str):
            line (str):
            state (str):
            tenant_id (str):
            acquired_at (datetime.datetime):
            recording_enabled (bool):
            block_anonymous (bool):
            tags (list[str]):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            e911_address (E911AddressResponse | None | Unset):
            e911_callback (None | str | Unset):
            cnam (None | str | Unset):
            spam_score (float | None | Unset):
            location_id (None | str | Unset):
            site_id (None | str | Unset): The site whose phones answer this number, in a hybrid deployment
                (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
            carrier (None | str | Unset):
            lrn (None | str | Unset):
            rate_center (None | str | Unset):
            activated_at (datetime.datetime | None | Unset):
     """

    id: str
    e164: str
    country_code: str
    npa: str
    nxx: str
    line: str
    state: str
    tenant_id: str
    acquired_at: datetime.datetime
    recording_enabled: bool
    block_anonymous: bool
    tags: list[str]
    created_at: datetime.datetime
    updated_at: datetime.datetime
    e911_address: E911AddressResponse | None | Unset = UNSET
    e911_callback: None | str | Unset = UNSET
    cnam: None | str | Unset = UNSET
    spam_score: float | None | Unset = UNSET
    location_id: None | str | Unset = UNSET
    site_id: None | str | Unset = UNSET
    carrier: None | str | Unset = UNSET
    lrn: None | str | Unset = UNSET
    rate_center: None | str | Unset = UNSET
    activated_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        id = self.id

        e164 = self.e164

        country_code = self.country_code

        npa = self.npa

        nxx = self.nxx

        line = self.line

        state = self.state

        tenant_id = self.tenant_id

        acquired_at = self.acquired_at.isoformat()

        recording_enabled = self.recording_enabled

        block_anonymous = self.block_anonymous

        tags = self.tags



        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        e911_address: dict[str, Any] | None | Unset
        if isinstance(self.e911_address, Unset):
            e911_address = UNSET
        elif isinstance(self.e911_address, E911AddressResponse):
            e911_address = self.e911_address.to_dict()
        else:
            e911_address = self.e911_address

        e911_callback: None | str | Unset
        if isinstance(self.e911_callback, Unset):
            e911_callback = UNSET
        else:
            e911_callback = self.e911_callback

        cnam: None | str | Unset
        if isinstance(self.cnam, Unset):
            cnam = UNSET
        else:
            cnam = self.cnam

        spam_score: float | None | Unset
        if isinstance(self.spam_score, Unset):
            spam_score = UNSET
        else:
            spam_score = self.spam_score

        location_id: None | str | Unset
        if isinstance(self.location_id, Unset):
            location_id = UNSET
        else:
            location_id = self.location_id

        site_id: None | str | Unset
        if isinstance(self.site_id, Unset):
            site_id = UNSET
        else:
            site_id = self.site_id

        carrier: None | str | Unset
        if isinstance(self.carrier, Unset):
            carrier = UNSET
        else:
            carrier = self.carrier

        lrn: None | str | Unset
        if isinstance(self.lrn, Unset):
            lrn = UNSET
        else:
            lrn = self.lrn

        rate_center: None | str | Unset
        if isinstance(self.rate_center, Unset):
            rate_center = UNSET
        else:
            rate_center = self.rate_center

        activated_at: None | str | Unset
        if isinstance(self.activated_at, Unset):
            activated_at = UNSET
        elif isinstance(self.activated_at, datetime.datetime):
            activated_at = self.activated_at.isoformat()
        else:
            activated_at = self.activated_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "e164": e164,
            "country_code": country_code,
            "npa": npa,
            "nxx": nxx,
            "line": line,
            "state": state,
            "tenant_id": tenant_id,
            "acquired_at": acquired_at,
            "recording_enabled": recording_enabled,
            "block_anonymous": block_anonymous,
            "tags": tags,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if e911_address is not UNSET:
            field_dict["e911_address"] = e911_address
        if e911_callback is not UNSET:
            field_dict["e911_callback"] = e911_callback
        if cnam is not UNSET:
            field_dict["cnam"] = cnam
        if spam_score is not UNSET:
            field_dict["spam_score"] = spam_score
        if location_id is not UNSET:
            field_dict["location_id"] = location_id
        if site_id is not UNSET:
            field_dict["site_id"] = site_id
        if carrier is not UNSET:
            field_dict["carrier"] = carrier
        if lrn is not UNSET:
            field_dict["lrn"] = lrn
        if rate_center is not UNSET:
            field_dict["rate_center"] = rate_center
        if activated_at is not UNSET:
            field_dict["activated_at"] = activated_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        e164 = d.pop("e164")

        country_code = d.pop("country_code")

        npa = d.pop("npa")

        nxx = d.pop("nxx")

        line = d.pop("line")

        state = d.pop("state")

        tenant_id = d.pop("tenant_id")

        acquired_at = datetime.datetime.fromisoformat(d.pop("acquired_at"))




        recording_enabled = d.pop("recording_enabled")

        block_anonymous = d.pop("block_anonymous")

        tags = cast(list[str], d.pop("tags"))


        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_e911_address(data: object) -> E911AddressResponse | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                e911_address_type_0 = E911AddressResponse.from_dict(data)



                return e911_address_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(E911AddressResponse | None | Unset, data)

        e911_address = _parse_e911_address(d.pop("e911_address", UNSET))


        def _parse_e911_callback(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        e911_callback = _parse_e911_callback(d.pop("e911_callback", UNSET))


        def _parse_cnam(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        cnam = _parse_cnam(d.pop("cnam", UNSET))


        def _parse_spam_score(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        spam_score = _parse_spam_score(d.pop("spam_score", UNSET))


        def _parse_location_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        location_id = _parse_location_id(d.pop("location_id", UNSET))


        def _parse_site_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        site_id = _parse_site_id(d.pop("site_id", UNSET))


        def _parse_carrier(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        carrier = _parse_carrier(d.pop("carrier", UNSET))


        def _parse_lrn(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        lrn = _parse_lrn(d.pop("lrn", UNSET))


        def _parse_rate_center(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        rate_center = _parse_rate_center(d.pop("rate_center", UNSET))


        def _parse_activated_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                activated_at_type_0 = datetime.datetime.fromisoformat(data)



                return activated_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        activated_at = _parse_activated_at(d.pop("activated_at", UNSET))


        page_phone_number_response_data_item = cls(
            id=id,
            e164=e164,
            country_code=country_code,
            npa=npa,
            nxx=nxx,
            line=line,
            state=state,
            tenant_id=tenant_id,
            acquired_at=acquired_at,
            recording_enabled=recording_enabled,
            block_anonymous=block_anonymous,
            tags=tags,
            created_at=created_at,
            updated_at=updated_at,
            e911_address=e911_address,
            e911_callback=e911_callback,
            cnam=cnam,
            spam_score=spam_score,
            location_id=location_id,
            site_id=site_id,
            carrier=carrier,
            lrn=lrn,
            rate_center=rate_center,
            activated_at=activated_at,
        )


        page_phone_number_response_data_item.additional_properties = d
        return page_phone_number_response_data_item

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
