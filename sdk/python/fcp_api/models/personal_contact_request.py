from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.contact_number import ContactNumber





T = TypeVar("T", bound="PersonalContactRequest")



@_attrs_define
class PersonalContactRequest:
    """ A contact, as written.

        Attributes:
            name (str):
            company (None | str | Unset):
            numbers (list[ContactNumber] | Unset):
            email (None | str | Unset):
            notes (None | str | Unset):
            ringtone (None | str | Unset): The ringtone the user's apps ring with for this contact, by the
                app's id for it; `null` rings with the app's own choice, and left
                out a change keeps the one set (an app or page that has none to
                offer does not clear it).
     """

    name: str
    company: None | str | Unset = UNSET
    numbers: list[ContactNumber] | Unset = UNSET
    email: None | str | Unset = UNSET
    notes: None | str | Unset = UNSET
    ringtone: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.contact_number import ContactNumber # noqa: PLC0415
        name = self.name

        company: None | str | Unset
        if isinstance(self.company, Unset):
            company = UNSET
        else:
            company = self.company

        numbers: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.numbers, Unset):
            numbers = []
            for numbers_item_data in self.numbers:
                numbers_item = numbers_item_data.to_dict()
                numbers.append(numbers_item)



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
            "name": name,
        })
        if company is not UNSET:
            field_dict["company"] = company
        if numbers is not UNSET:
            field_dict["numbers"] = numbers
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
        name = d.pop("name")

        def _parse_company(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        company = _parse_company(d.pop("company", UNSET))


        _numbers = d.pop("numbers", UNSET)
        numbers: list[ContactNumber] | Unset = UNSET
        if _numbers is not UNSET:
            numbers = []
            for numbers_item_data in _numbers:
                numbers_item = ContactNumber.from_dict(numbers_item_data)



                numbers.append(numbers_item)


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


        personal_contact_request = cls(
            name=name,
            company=company,
            numbers=numbers,
            email=email,
            notes=notes,
            ringtone=ringtone,
        )


        personal_contact_request.additional_properties = d
        return personal_contact_request

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
