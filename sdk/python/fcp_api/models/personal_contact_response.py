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
  from ..models.contact_number import ContactNumber





T = TypeVar("T", bound="PersonalContactResponse")



@_attrs_define
class PersonalContactResponse:
    """ A contact.

        Attributes:
            id (str):
            name (str):
            numbers (list[ContactNumber]):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            company (None | str | Unset):
            email (None | str | Unset):
            notes (None | str | Unset):
            ringtone (None | str | Unset):
     """

    id: str
    name: str
    numbers: list[ContactNumber]
    created_at: datetime.datetime
    updated_at: datetime.datetime
    company: None | str | Unset = UNSET
    email: None | str | Unset = UNSET
    notes: None | str | Unset = UNSET
    ringtone: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.contact_number import ContactNumber # noqa: PLC0415
        id = self.id

        name = self.name

        numbers = []
        for numbers_item_data in self.numbers:
            numbers_item = numbers_item_data.to_dict()
            numbers.append(numbers_item)



        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        company: None | str | Unset
        if isinstance(self.company, Unset):
            company = UNSET
        else:
            company = self.company

        email: None | str | Unset
        if isinstance(self.email, Unset):
            email = UNSET
        else:
            email = self.email

        notes: None | str | Unset
        if isinstance(self.notes, Unset):
            notes = UNSET
        else:
            notes = self.notes

        ringtone: None | str | Unset
        if isinstance(self.ringtone, Unset):
            ringtone = UNSET
        else:
            ringtone = self.ringtone


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "name": name,
            "numbers": numbers,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if company is not UNSET:
            field_dict["company"] = company
        if email is not UNSET:
            field_dict["email"] = email
        if notes is not UNSET:
            field_dict["notes"] = notes
        if ringtone is not UNSET:
            field_dict["ringtone"] = ringtone

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.contact_number import ContactNumber # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        name = d.pop("name")

        numbers = []
        _numbers = d.pop("numbers")
        for numbers_item_data in (_numbers):
            numbers_item = ContactNumber.from_dict(numbers_item_data)



            numbers.append(numbers_item)


        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_company(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        company = _parse_company(d.pop("company", UNSET))


        def _parse_email(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        email = _parse_email(d.pop("email", UNSET))


        def _parse_notes(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        notes = _parse_notes(d.pop("notes", UNSET))


        def _parse_ringtone(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        ringtone = _parse_ringtone(d.pop("ringtone", UNSET))


        personal_contact_response = cls(
            id=id,
            name=name,
            numbers=numbers,
            created_at=created_at,
            updated_at=updated_at,
            company=company,
            email=email,
            notes=notes,
            ringtone=ringtone,
        )


        personal_contact_response.additional_properties = d
        return personal_contact_response

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
